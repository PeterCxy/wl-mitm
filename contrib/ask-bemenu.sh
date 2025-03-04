#!/usr/bin/env bash

prompt="$3"

if [ -z "$prompt" ]; then
  prompt="calling $1::$2?"
fi

res=$(echo -e "yes\nno" | bemenu -l 2 -s --prompt "Allow $prompt?")

if [ "$res" == "yes" ]; then
  exit 0
else
  exit 1
fi
