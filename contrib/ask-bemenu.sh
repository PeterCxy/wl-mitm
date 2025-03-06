#!/usr/bin/env bash

prompt="$3"

if [ -z "$prompt" ]; then
  prompt="calling $1::$2?"
fi

if [ ! -z "$WL_MITM_LAST_TOPLEVEL_TITLE" ]; then
  prompt="\"$WL_MITM_LAST_TOPLEVEL_TITLE\" $prompt"
elif [ ! -z "$WL_MITM_LAST_TOPLEVEL_APP_ID" ]; then
  prompt="\"$WL_MITM_LAST_TOPLEVEL_APP_ID\" $prompt"
fi

res=$(echo -e "yes\nno" | bemenu -l 2 -s --prompt "Allow $prompt?")

if [ "$res" == "yes" ]; then
  exit 0
else
  exit 1
fi
