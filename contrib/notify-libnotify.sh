#!/usr/bin/env bash

msg="$3"

if [ -z "$msg" ]; then
  msg="called $1::$2"
fi

notify-send "$msg"
