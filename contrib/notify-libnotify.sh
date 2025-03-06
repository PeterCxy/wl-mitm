#!/usr/bin/env bash

msg="$3"

if [ -z "$msg" ]; then
  msg="called $1::$2"
fi

if [ ! -z "$WL_MITM_LAST_TOPLEVEL_TITLE" ]; then
  msg="\"$WL_MITM_LAST_TOPLEVEL_TITLE\" $msg"
elif [ ! -z "$WL_MITM_LAST_TOPLEVEL_APP_ID" ]; then
  msg="\"$WL_MITM_LAST_TOPLEVEL_APP_ID\" $msg"
fi

notify-send "$msg"
