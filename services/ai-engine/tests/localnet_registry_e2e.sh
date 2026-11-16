#!/usr/bin/env bash
# Audit A3: the engine reads the asset universe from the REAL registry
# contract on a local Stellar network (quickstart, release WASM).
# Usage (repo root, after `stellar contract build`): services/ai-engine/tests/localnet_registry_e2e.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
NET=constella-ai-net; NODE=constella-ai-node
CLI_IMAGE=constella-stellar-build:30.0.1; QS_IMAGE=${QS_IMAGE:-stellar/quickstart:latest}
RPC=http://$NODE:8000/soroban/rpc; PASS="Standalone Network ; February 2017"
W=$ROOT/contracts/target/wasm32v1-none/release; WORK=$(mktemp -d)
trap 'docker rm -f $NODE >/dev/null 2>&1 || true; docker network rm $NET >/dev/null 2>&1 || true; rm -rf "$WORK"' EXIT
fail() { echo "E2E FAIL: $*" >&2; exit 1; }
[ -f "$W/constella_registry.wasm" ] || fail "build the contracts first"
docker network create $NET >/dev/null 2>&1 || true
docker run -d --rm --name $NODE --network $NET $QS_IMAGE --local --enable core,rpc >/dev/null
for i in $(seq 1 120); do
  docker run --rm --network $NET curlimages/curl:8.10.1 -s -X POST $RPC -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' 2>/dev/null | grep -q '"healthy"' && break
  sleep 5; [ "$i" = 120 ] && fail "RPC never became healthy"
done
U="$(id -u):$(id -g)"
cli() { local pre=() post=() seen=0; for a in "$@"; do if [ $seen = 0 ] && [ "$a" = "--" ]; then seen=1; fi
          if [ $seen = 1 ]; then post+=("$a"); else pre+=("$a"); fi; done
        docker run --rm --user "$U" --network $NET -v "$WORK:/work" -v "$W:/wasm:ro" -e STELLAR_CONFIG_HOME=/work -e HOME=/work \
          $CLI_IMAGE stellar "${pre[@]}" --rpc-url $RPC --network-passphrase "$PASS" "${post[@]}"; }
keys() { docker run --rm --user "$U" --network $NET -v "$WORK:/work" -e STELLAR_CONFIG_HOME=/work -e HOME=/work $CLI_IMAGE stellar keys "$@"; }
fund() { for i in $(seq 1 30); do
    [ "$(docker run --rm --network $NET curlimages/curl:8.10.1 -s -o /dev/null -w "%{http_code}" "http://$NODE:8000/friendbot?addr=$1")" = 200 ] && return 0; sleep 5
  done; fail "friendbot could not fund $1"; }
for k in admin issuer; do keys generate $k >/dev/null 2>&1; fund "$(keys address $k)"; done
ADMIN=$(keys address admin); ISSUER=$(keys address issuer)
sac() { local id; id=$(cli contract asset deploy --asset "$1:$ISSUER" --source admin 2>"$WORK/err" | tail -1); [[ $id == C* ]] || fail "SAC $1: $(tail -2 "$WORK/err")"; echo "$id"; }
USDC=$(sac USDC); BLND=$(sac BLNDPOS); BENJI=$(sac BENJI); MYST=$(sac MYSTERY)
REGISTRY=$(cli contract deploy --wasm /wasm/constella_registry.wasm --source admin -- --admin $ADMIN 2>"$WORK/err" | tail -1); [[ $REGISTRY == C* ]] || fail "registry: $(tail -2 "$WORK/err")"
add() { cli contract invoke --id $REGISTRY --source admin -- add_asset --entry "$1" >/dev/null 2>"$WORK/err" || fail "add_asset: $(tail -2 "$WORK/err")"; }
j() { echo "{\"contract_address\":\"$1\",\"asset_id\":\"$2\",\"issuer\":\"$3\",\"sleeve_type\":$4,\"liquidity_tier\":$5,\"permissionless\":$6,\"oracle_sources\":[\"SRC\"],\"eligible_jurisdictions\":[],\"blocked_jurisdictions\":$7,\"investor_classes_required\":[],\"is_active\":true,\"added_at\":0}"; }
add "$(j $USDC USDC CIRCLE 0 0 true '[]')"
add "$(j $BLND BLEND_USDC_SUPPLY BLEND 4 1 true '[]')"
add "$(j $BENJI FRANKLIN_BENJI FRANKLIN_TEMPLETON 1 1 false '["CN"]')"
add "$(j $MYST MYSTERY NOBODY 0 0 true '[]')"
cli contract invoke --id $REGISTRY --source admin -- deactivate_asset --asset_id BLEND_USDC_SUPPLY >/dev/null 2>"$WORK/err" || fail "deactivate: $(tail -2 "$WORK/err")"
echo "registry $REGISTRY with USDC, BLEND (deactivated), BENJI (gated, CN blocked), MYSTERY (undescribed)"
docker run --rm --network $NET -v "$ROOT/services/ai-engine:/svc" -w /svc -e RPC=$RPC -e "PASS=$PASS" -e REGISTRY=$REGISTRY \
  -e VIEWER=$ADMIN -e USDC=$USDC python:3.12-slim \
  sh -c 'pip install -q --no-cache-dir --root-user-action=ignore -e . >/dev/null 2>&1; python -m tests.localnet_registry_check' || fail "engine check"
echo "E2E PASS"
