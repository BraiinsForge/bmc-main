#!/usr/bin/env bash

if [[ -x /usr/bin/just ]]; then
  eval "$(/usr/bin/just --completions bash 2>/dev/null)" || true
fi
