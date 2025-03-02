#!/usr/bin/env bash

res=$(echo -e "yes\nno" | bemenu -l 2 --prompt "Allow calling $1::$2?")

if [ "$res" == "yes" ]; then
  exit 0
else
  exit 1
fi
