#!/usr/bin/env bash
# Stop everything and forget the local chain (the next up.sh deploys fresh).
cd "$(dirname "$0")" && docker compose down -v --remove-orphans && rm -rf state
