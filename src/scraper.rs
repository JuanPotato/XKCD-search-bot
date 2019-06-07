use futures::channel::mpsc;
use futures::compat::{Future01CompatExt, Stream01CompatExt};
use futures::{FutureExt, SinkExt, StreamExt, TryFutureExt};

use reqwest::r#async::{Chunk, Client, Response};
use select::document::Document;
use select::predicate::{Class, Name, Predicate};
use serde_derive::{Deserialize, Serialize};

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Read, Write, Seek};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
pub struct XkcdInfo {
    pub num: usize,

    #[serde(rename="safe_title")]
    pub title: String,
    pub img: String,
    pub alt: String,
    pub transcript: String,

    pub year: String,
    pub month: String,
    pub day: String,
}

pub async fn update_comics(comic_path: &str) -> HashMap<String, XkcdInfo> {
    let mut xkcd_comics_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(comic_path)
        .unwrap();

    let mut xkcd_comics_str = String::new();
    xkcd_comics_file.read_to_string(&mut xkcd_comics_str).unwrap();

    let mut xkcd_comics: HashMap<String, XkcdInfo> = serde_json::from_str(&xkcd_comics_str)
        .unwrap_or_default();


    let arc_client = Arc::new(Client::new());

    let latest_comic_num = get_raw_xkcd_info(&arc_client, 0).await.num;
    let last_scrapped_comic = xkcd_comics.keys().map(|s| {
        s.parse::<usize>().unwrap()
    }).max().unwrap_or(1);

    let comic_start: usize = last_scrapped_comic.checked_sub(6).unwrap_or(0) + 1;

    xkcd_comics = scrape_comics(arc_client, comic_start..latest_comic_num, xkcd_comics).await;

    let output = serde_json::to_string_pretty(&xkcd_comics).unwrap();

    xkcd_comics_file.seek(std::io::SeekFrom::Start(0)).unwrap();
    xkcd_comics_file.set_len(0).unwrap();
    xkcd_comics_file.write_all(output.as_bytes()).unwrap();

    xkcd_comics
}


async fn scrape_comics(
    client: Arc<Client>,
    comic_nums: impl Iterator<Item=usize>,
    mut xkcd_comics: HashMap<String, XkcdInfo>
) -> HashMap<String, XkcdInfo> {
    let mut xkcd_nums = comic_nums.filter(|&n| n != 404); // remove 404 if found

    const MAX_WORKERS: usize = 250;

    let (mut comic_result_tx, mut comic_result_rx) = mpsc::channel(MAX_WORKERS);
    let mut workers_tx = Vec::new();

    for id in 0..MAX_WORKERS {
        if let Some(xkcd_num) = xkcd_nums.next() {
            let (mut worker_tx, worker_rx) = mpsc::channel(1);

            let worker = xkcd_scraper(client.clone(), id, comic_result_tx.clone(), worker_rx);
            worker_tx.send(xkcd_num).await.unwrap();
            workers_tx.push(worker_tx);

            tokio::spawn(worker.boxed().unit_error().compat());
        } else {
            break;
        }
    }

    comic_result_tx.disconnect(); // Kill our local sender

    while let Some((info, worker_id)) = comic_result_rx.next().await {
        xkcd_comics.insert(info.num.to_string(), info);

        if let Some(xkcd_num) = xkcd_nums.next() {
            workers_tx[worker_id].send(xkcd_num).await.unwrap();
        } else {
            workers_tx[worker_id].disconnect();
        }
    }

    xkcd_comics
}

async fn xkcd_scraper(
    client: Arc<Client>,
    id: usize,
    mut result_tx: mpsc::Sender<(XkcdInfo, usize)>,
    mut job_rx: mpsc::Receiver<usize>,
) {
    while let Some(xkcd_num) = job_rx.next().await {
        let xkcd_info = get_xkcd_info(&client, xkcd_num).await;
        result_tx.send((xkcd_info, id)).await.unwrap();
    }
}

pub async fn get_xkcd_info(client: &Arc<Client>, xkcd_num: usize) -> XkcdInfo {
    let mut xkcd_info = get_raw_xkcd_info(&client, xkcd_num).await;
    xkcd_info.transcript = get_xkcd_transcript(&client, xkcd_num)
        .await
        .trim()
        .to_owned();

    println!("{}: {:?}", xkcd_num, xkcd_info);

    xkcd_info
}

async fn get_xkcd_transcript_html(client: &Arc<Client>, xkcd_num: usize) -> String {
    let url = format!("https://www.explainxkcd.com/wiki/index.php/{}", xkcd_num);

    let resp: Response = client
        .get(&url)
        .send()
        .compat()
        .await
        .expect("Could not send explainxkcd request.");

    let body: Chunk = resp
        .into_body()
        .compat()
        .map(|c| c.expect("Could not read xkcd transcript chunk."))
        .concat()
        .await;

    String::from_utf8(body.to_vec()).expect("Could not parse xkcd transcript response.")
}

async fn get_xkcd_transcript(client: &Arc<Client>, xkcd_num: usize) -> String {
    let html_str = get_xkcd_transcript_html(client, xkcd_num).await;

    let doc = Document::from(html_str.as_str());

    let transcript_selector = Name("h2").descendant(Name("span").and(Class("mw-headline")));

    let transcript_span = doc
        .find(transcript_selector)
        .filter(|e| {
            e.attr("id")
                .unwrap_or("")
                .to_ascii_lowercase()
                .contains("transcript")
        })
        .next();

    if transcript_span.is_none() {
        return String::new();
    }

    let mut transcript_node = transcript_span.unwrap().parent().expect(&format!(
        "{}: Could not get parent of transcript node",
        xkcd_num
    ));

    let mut transcript = String::new();

    while let Some(elem) = transcript_node.next() {
        if elem.name() == Some("h2") || elem.attr("id").is_some() {
            break;
        }

        transcript.push_str(&elem.text());
        transcript_node = elem;
    }

    transcript
}

async fn get_raw_xkcd_info(client: &Arc<Client>, xkcd_num: usize) -> XkcdInfo {
    let url = if xkcd_num == 0 {
        "https://www.xkcd.com/info.0.json".into()
    } else {
        format!("https://www.xkcd.com/{}/info.0.json", xkcd_num)
    };

    let mut resp: Response = client
        .get(&url)
        .send()
        .compat()
        .await
        .expect("Error receiving xkcd info json response.");

    resp.json()
        .compat()
        .await
        .expect("Could not read xkcd info json.")
}
