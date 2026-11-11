#!/bin/sh
BOT={{rule:telegram-bot-token}}
curl -s "https://api.telegram.org/bot${BOT}/sendMessage" -d chat_id=1 -d text=done
