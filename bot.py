#!/usr/bin/env python3

from aiogram import Bot, Dispatcher, executor, types
from aiogram.types import ChatType
from unidecode import unidecode
from fuzzywuzzy import process
import logging
import json
import html
import re


logging.basicConfig(level=logging.INFO)


API_TOKEN = '217427778:AAFwt1LfOsMfPD_I_lX4hZQpBMQdPuLGTf8'
USERNAME = 'xtbot'

bot = Bot(token=API_TOKEN, parse_mode='html')
dp = Dispatcher(bot)


def normalize(text):
    return unidecode(text.lower())


class XKCDSearch:
    def __init__(self):
        with open('xkcd.txt', 'r') as xkcd_file:
            self.comics = json.load(xkcd_file)

        self.search_list = []
        for c in self.comics.values():
            search_text = f'{c["num"]}\n{normalize(c["title"])}\n{normalize(c["alt"])}'
            self.search_list.append(search_text)

    def search(self, term):
        results = []

        for res in process.extract(normalize(term), self.search_list, limit=10):
            results.append(res[0].split('\n', 1)[0])

        try:
            int(term)
            if term in results:
                results.remove(term)
            if term in self.comics:
                results.insert(0, term)
        except ValueError:
            pass

        return results


xkcd = XKCDSearch()


@dp.message_handler(regexp=f'^/(start|help)(@{USERNAME})?')
async def help_start(message: types.Message):
    await bot.send_message(message.chat.id, 'Nice',
                           reply_to_message_id=message.message_id)


async def get_latest():
    return ['123']


@dp.inline_handler()
async def inline_xkcd(inline_query: types.InlineQuery):
    input_content = inline_query.query
    if not input_content:
        res = await get_latest()
    else:
        res = xkcd.search(input_content)

    items = []
    for xkcd_id in res:
        comic = xkcd.comics[xkcd_id]
        link = f'https://xkcd.com/{comic["num"]}/'
        title = comic['safe_title'] if 'safe_title' in comic else comic['title']
        alt = comic['alt']

        text = f'<a href="{link}">{xkcd_id}</a>: <b>{html.escape(title)}</b>\n\n<i>{html.escape(alt)}</i>'

        items.append(
            types.InlineQueryResultArticle(
                id=str(xkcd_id),
                title=f'{title}',
                url=link,
                input_message_content=types.InputTextMessageContent(text))
        )

    await bot.answer_inline_query(inline_query.id, results=items, cache_time=1)


@dp.message_handler()
async def messages(message: types.Message):
    if not message.chat.type == ChatType.PRIVATE:
        message.chat.leave()


if __name__ == '__main__':
    executor.start_polling(dp, skip_updates=True)
