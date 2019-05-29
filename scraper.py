from bs4 import BeautifulSoup, NavigableString
import aiohttp
import asyncio
import json
import re
import time

XKCD_JSON = "https://xkcd.com/{num}/info.0.json"
XKCD_EXPLAIN = "https://explainxkcd.com/{num}"


async def fetch(session, url):
    async with session.get(url) as response:
        return await response.text()


async def get_xkcd_json(session, num=None):
    print(f'JSON {num}')
    text = await fetch(session, XKCD_JSON.format(num=num or ""))
    return json.loads(text)


def get_text(elem):
    if isinstance(elem, NavigableString):
        return str(elem)
    try:
        return elem.get_text()
    except Exception:
        return ""


def is_boundary(elem):
    try:
        if elem.name == "h2":
            return True
    except Exception:
        pass

    try:
        if elem["id"] in (
            "Discussion",
            "Trivia",
            "-.2A-.2A-.2A.26.26.26------DiScUsSiOn------.26.26.26.2A-.2A-.2A",
            ".7E.2A.7ETrIvIa.7E.2A.7E",
        ):
            return True
    except Exception:
        pass

    return False


async def get_xkcd_transcript(session, num):
    text = await fetch(session, XKCD_EXPLAIN.format(num=num))
    soup = BeautifulSoup(text, "html5lib")
    transcript = ""

    transcript_node = soup.find(id="Transcript") or soup.find(
        id=".7E.2A.7ETrAnScRiPt.7E.2A.7E"
    )

    if transcript_node is None:
        return None

    transcript_node = transcript_node.parent.next_sibling

    while not is_boundary(transcript_node):
        transcript += get_text(transcript_node)
        transcript_node = transcript_node.next_sibling

    return transcript.strip()


async def download_xkcd(session, num):
    print(f'Downloading {num}')

    if num == 404:
        # transcript = 'Error: 404. Comic not found.'
        json = {
            'num': 404,
            'title': '404',
            'safe_title': '404',
            'alt': '',
        }
    else:
        try:
            json = await get_xkcd_json(session, num)
            # transcript = await get_xkcd_transcript(session, num)
            # if transcript is None:
            #    transcript = json["transcript"]
        except Exception:
            return f'Error - {num}'

    with open(f"./small_xkcd_comics/xkcd_{num:05}.txt", "w+") as out_file:
        out_file.write(f'{json["num"]}\n{json["title"]}\n{json["alt"]}\n\n')
        # out_file.write(transcript)
        print(f"Finished {num}")

    return json


async def download_all_xkcd():
    async with aiohttp.ClientSession() as session:
        latest_num = (await get_xkcd_json(session))["num"]

        comic_range = range(1, latest_num + 1)

        done, pending = await asyncio.wait([download_xkcd(session, num) for num in comic_range])
        results = [f.result() for f in done]
        comic_map = {str(r['num']): r for r in results}

        with open(f"./xkcd.txt", "w+") as out_file:
            out_file.write(json.dumps(comic_map))



loop = asyncio.get_event_loop()
loop.run_until_complete(download_all_xkcd())
