#!/usr/bin/env bash
# One command: local Stellar network + every Constella contract + relayer,
# keeper, watchdog, thesis API and a mail sink. Then: cd sandbox && npm run dev.
# Requires the release WASMs: run `stellar contract build` first.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p state
[ -f state/services.env ] || : > state/services.env   # compose needs the file before the first deploy
docker compose up -d node
echo "waiting for the local RPC..."
for i in $(seq 1 120); do
  curl -s -X POST localhost:8000/soroban/rpc -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' 2>/dev/null | grep -q healthy && break
  sleep 3; [ "$i" = 120 ] && { echo "RPC never became healthy"; exit 1; }
done
docker run --rm --network constella-local_default -v "$(cd .. && pwd):/repo" -w /repo \
  -e RPC_URL=http://node:8000/soroban/rpc -e FRIENDBOT_URL=http://node:8000/friendbot python:3.12-slim \
  sh -c "pip install -q --no-cache-dir --root-user-action=ignore stellar-sdk==17.0.1 >/dev/null 2>&1 && python localnet/deploy.py && python localnet/render.py"
docker compose up -d relayer keeper watchdog thesis-api mailsink
echo "stack up: RPC :8000  relayer :8787  thesis API :8790   (logs: docker compose -f localnet/docker-compose.yml logs -f)"
echo "console:  cd sandbox && npm install && npm run dev  ->  http://localhost:5173"
