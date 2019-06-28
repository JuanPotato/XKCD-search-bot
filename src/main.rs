/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

#![feature(async_await)]

use std::sync::Arc;
use std::time::Duration;

use futures::{FutureExt, StreamExt, TryFutureExt};
use futures_timer::Interval;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{ReloadPolicy, IndexReader, IndexWriter};
use tantivy::Index;

mod scraper;

use tg_botapi::Bot;
use tg_botapi::methods::AnswerInlineQuery;
use tg_botapi::types::{UpdateType, InlineQuery, InputTextMessageContent,
                       ParseMode, InlineQueryResultArticle};

fn main() {
    tokio::run(async_main().map(Result::unwrap).boxed().unit_error().compat());
}

struct XkcdBot {
    api: Bot,
    num_field: Field,
    title_field: Field,
    alt_field: Field,
    img_field: Field,
    reader: IndexReader,
    query_parser: QueryParser,
}

async fn async_main() -> tantivy::Result<()> {
    let mut schema_builder = Schema::builder();
    schema_builder.add_text_field("num", STRING | STORED);
    schema_builder.add_text_field("title", TEXT | STORED);
    schema_builder.add_text_field("alt", TEXT | STORED);
    schema_builder.add_text_field("transcript", TEXT);
    schema_builder.add_text_field("img", TEXT | STORED);

    let schema = schema_builder.build();
    let index = Index::create_in_ram(schema.clone());

    let index_writer = index.writer(50_000_000)?;

    let num = schema.get_field("num").unwrap();
    let title = schema.get_field("title").unwrap();
    let alt = schema.get_field("alt").unwrap();
    let transcript = schema.get_field("transcript").unwrap();
    let img = schema.get_field("img").unwrap();

    let mut index_updater = IndexUpdater {
        index_writer: index_writer,
        num_field: num.clone(),
        title_field: title.clone(),
        alt_field: alt.clone(),
        transcript_field: transcript.clone(),
        img_field: img.clone(),
    };

    index_updater.update(true).await?;

    tokio::spawn(async move {
        let mut interval = Interval::new(Duration::from_secs(24*60*60));
        while let Some(_) = interval.next().await {
            index_updater.update(false).await.unwrap();
        }
    }.boxed().unit_error().compat());

    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::OnCommit)
        .try_into()?;

    let query_parser = QueryParser::for_index(&index, vec![title, alt, transcript]);

    let token = "217427778:nice_try";
    let bot = Arc::new(XkcdBot {
        api: Bot::new(token),
        num_field: num,
        title_field: title,
        alt_field: alt,
        img_field: img,
        reader: reader,
        query_parser: query_parser,
    });

    let mut updates = bot.api.start_polling();

    while let Some(update) = updates.next().await {
        if let UpdateType::InlineQuery(query) = update.update_type {
            tokio::spawn(
                handle_inline_query(bot.clone(), query)
                    .boxed()
                    .unit_error()
                    .compat(),
            );
        }
    }

    Ok(())
}

struct IndexUpdater {
    index_writer: IndexWriter,
    num_field: Field,
    title_field: Field,
    alt_field: Field,
    transcript_field: Field,
    img_field: Field,
}

impl IndexUpdater {
    async fn update(&mut self, initial_load: bool) -> tantivy::Result<()> {
        let (latest_num, xkcd_comics) = scraper::update_comics("xkcd.json").await;

        if initial_load {
            for (comic_num, comic) in xkcd_comics.iter() {
                let mut comic_doc = Document::default();
                comic_doc.add_text(self.num_field, &comic_num);
                comic_doc.add_text(self.title_field, &comic.title);
                comic_doc.add_text(self.alt_field, &comic.alt);
                comic_doc.add_text(self.transcript_field, &comic.transcript);
                comic_doc.add_text(self.img_field, &comic.img);

                self.index_writer.add_document(comic_doc);
            }
        } else {
            for xkcd_num in latest_num-3 ..= latest_num {
                let xkcd_str = format!("{}", xkcd_num);
                let xkcd_term = Term::from_field_text(self.num_field, &xkcd_str);
                self.index_writer.delete_term(xkcd_term);

                let comic = xkcd_comics.get(&xkcd_str).unwrap();
                let mut comic_doc = Document::default();
                comic_doc.add_text(self.num_field, &xkcd_str);
                comic_doc.add_text(self.title_field, &comic.title);
                comic_doc.add_text(self.alt_field, &comic.alt);
                comic_doc.add_text(self.transcript_field, &comic.transcript);
                comic_doc.add_text(self.img_field, &comic.img);

                self.index_writer.add_document(comic_doc);
            }
        }

        self.index_writer.commit()?;

        Ok(())
    }
}

async fn handle_inline_query(bot: Arc<XkcdBot>, query: InlineQuery) {
    if query.query.is_empty() {
        return;
    }

    let searcher = bot.reader.searcher();

    let mut response = AnswerInlineQuery::new(query.id, Vec::new());

    match bot.query_parser.parse_query(&query.query) {
        Ok(search_query) => {
            let top_docs = searcher.search(&search_query, &TopDocs::with_limit(15)).unwrap();

            for (_score, doc_address) in top_docs {
                let retrieved_doc = searcher.doc(doc_address).unwrap();
                let comic_num = retrieved_doc.get_first(bot.num_field).unwrap().text().unwrap();
                let comic_title = retrieved_doc.get_first(bot.title_field).unwrap().text().unwrap();
                let comic_alt = retrieved_doc.get_first(bot.alt_field).unwrap().text().unwrap();
                let comic_img = retrieved_doc.get_first(bot.img_field).unwrap().text().unwrap();

                let comic_text = format!(
                    "<a href=\"https://xkcd.com/{num}\">{num}</a>: <b>{title}</b>\n\n<i>{alt}</i>",
                    num=comic_num, title=html_escape(comic_title), alt=html_escape(comic_alt)
                );

                let article_title = format!("{}: {}", comic_num, comic_title);

                let mut content = InputTextMessageContent::new(comic_text);
                content.parse_mode = ParseMode::HTML;

                let mut article = InlineQueryResultArticle::new(comic_num, article_title, content);
                article.description = Some(comic_alt.into());
                article.thumb_url = Some(comic_img.into());
                response.add(article);
            }
        }

        Err(query_err) => {
            let err_str = format!("Error: {:?}", query_err);
            let content = InputTextMessageContent::new(err_str);
            response.add(InlineQueryResultArticle::new("err", "Query parsing error", content));
        }
    }

    response.cache_time = Some(0);
    response.is_personal = Some(false);

    bot.api.send(&response).await.unwrap();
}

fn html_escape(html: &str) -> String {
    html.replace('>', "&gt;").replace('<', "&lt;").replace('&', "&amp;")
}
