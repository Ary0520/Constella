#!/usr/bin/env bash
# End-to-end test of the feeder against a REAL local Stellar network
# (stellar/quickstart --local) running Constella's release oracle WASM.
# Usage (from repo root, after `stellar contract build`): services/feeder/tests/localnet_e2e.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
NET=constella-localnet
NODE=constella-stellar-local
CLI_IMAGE=constella-stellar-build:30.0.1
QS_IMAGE=${QS_IMAGE:-stellar/quickstart:latest}
RPC=http://$NODE:8000/soroban/rpc
PASS="Standalone Network ; February 2017"
WASM=$ROOT/contracts/target/wasm32v1-none/release/constella_nav_oracle.wasm
FIXTURE=/svc/tests/fixtures/ecb_eurofxref_daily_20261115.xml
WORK=$(mktemp -d)
trap 'docker rm -f $NODE >/dev/null 2>&1 || true; docker network rm $NET >/dev/null 2>&1 || true; rm -rf "$WORK"' EXIT

fail() { echo "E2E FAIL: $*" >&2; exit 1; }
[ -f "$WASM" ] || fail "build the contracts first ($WASM missing)"

docker network create $NET >/dev/null 2>&1 || true
docker rm -f $NODE >/dev/null 2>&1 || true
docker run -d --rm --name $NODE --network $NET $QS_IMAGE --local --enable core,rpc >/dev/null
echo "waiting for local RPC..."
for i in $(seq 1 120); do
  if docker run --rm --network $NET curlimages/curl:8.10.1 -s -X POST $RPC -H 'Content-Type: application/json' \
       -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' 2>/dev/null | grep -q '"healthy"'; then break; fi
  sleep 5; [ "$i" = 120 ] && fail "RPC never became healthy"
done

U="$(id -u):$(id -g)"
# network flags must precede `--` (everything after it is contract arguments)
cli() { local pre=() post=() seen=0; for a in "$@"; do if [ $seen = 0 ] && [ "$a" = "--" ]; then seen=1; fi
          if [ $seen = 1 ]; then post+=("$a"); else pre+=("$a"); fi; done
        docker run --rm --user "$U" --network $NET -v "$WORK:/work" -v "$WASM:/oracle.wasm:ro" -e STELLAR_CONFIG_HOME=/work -e HOME=/work \
          $CLI_IMAGE stellar "${pre[@]}" --rpc-url $RPC --network-passphrase "$PASS" "${post[@]}"; }
keys() { docker run --rm --user "$U" --network $NET -v "$WORK:/work" -e STELLAR_CONFIG_HOME=/work -e HOME=/work $CLI_IMAGE stellar keys "$@"; }
fund() {  # friendbot starts after RPC reports healthy: retry until it answers 200
  for i in $(seq 1 30); do
    code=$(docker run --rm --network $NET curlimages/curl:8.10.1 -s -o /dev/null -w "%{http_code}" "http://$NODE:8000/friendbot?addr=$1")
    [ "$code" = 200 ] && return 0; sleep 5
  done
  fail "friendbot could not fund $1"
}

for k in admin feeder issuer; do
  keys generate $k >/dev/null 2>&1
  addr=$(keys address $k)
  fund "$addr"
done
ADMIN=$(keys address admin); FEEDER=$(keys address feeder); ISSUER=$(keys address issuer)
FEEDER_SECRET=$(keys show feeder)

USDC=$(cli contract asset deploy --asset USDC:$ISSUER --source admin 2>"$WORK/err" | tail -1); [[ $USDC == C* ]] || fail "USDC deploy: $(tail -3 "$WORK/err")"
EURC=$(cli contract asset deploy --asset EURC:$ISSUER --source admin 2>"$WORK/err" | tail -1); [[ $EURC == C* ]] || fail "EURC deploy: $(tail -3 "$WORK/err")"
ORACLE=$(cli contract deploy --wasm /oracle.wasm --source admin -- --admin $ADMIN --base_asset $USDC --decimals 7 --resolution 300 2>"$WORK/err" | tail -1); [[ $ORACLE == C* ]] || fail "oracle deploy: $(tail -3 "$WORK/err")"
echo "oracle $ORACLE  usdc $USDC  eurc $EURC  (protocol: $(docker run --rm --network $NET curlimages/curl:8.10.1 -s -X POST $RPC -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"getVersionInfo"}' | grep -o '"protocolVersion":[0-9]*'))"

cli contract invoke --id $ORACLE --source admin -- configure_sleeve --sleeve_id CASH --deviation_threshold_bps 200 --min_valid_sources 2 >/dev/null 2>&1 || fail "configure_sleeve"
SRC="{\"source_id\":\"ECB_EURC\",\"asset\":\"$EURC\",\"sleeve\":\"CASH\",\"price\":\"0\",\"timestamp\":0,\"staleness_limit_seconds\":432000,\"min_liquidity_depth_usd\":\"0\",\"current_liquidity_depth_usd\":\"0\",\"is_active\":true,\"feeder\":\"$FEEDER\"}"
cli contract invoke --id $ORACLE --source admin -- register_source --source "$SRC" >/dev/null 2>&1 || fail "register_source"

feeder() { docker run --rm --network $NET -v "$ROOT/services/feeder:/svc" -v "$WORK:/tmpdata" -w /svc \
  -e CONSTELLA_NETWORK=local -e CONSTELLA_RPC_URL=$RPC -e CONSTELLA_ORACLE_ID=$ORACLE \
  -e FEEDER_SOURCE_ID=ECB_EURC -e FEEDER_SECRET="$FEEDER_SECRET" -e ECB_URL="$1" python:3.12-slim \
  sh -c 'pip install -q --no-cache-dir --root-user-action=ignore -e . >/dev/null 2>&1; constella-feeder'; }

echo "--- run 1 (expect SUBMIT)"
out=$(feeder file://$FIXTURE) || fail "first run failed: $out"; echo "$out"
echo "$out" | grep -q '^SUBMIT: ECB_EURC: ECB 2026-11-13 EUR/USD 1.1052 -> 11052000' || fail "unexpected: $out"
onchain=$(cli contract invoke --id $ORACLE --source admin --send=no -- get_source --source_id ECB_EURC 2>/dev/null | tail -1)
echo "on-chain: $onchain"
echo "$onchain" | grep -q '"price":"11052000"' || fail "price not on-chain"
echo "$onchain" | grep -q '"timestamp":1794575700' || fail "timestamp not the ECB concertation time (2026-11-13 14:15 CET)"

echo "--- run 2 (expect SKIP: same data already on-chain)"
out=$(feeder file://$FIXTURE) || fail "second run failed: $out"; echo "$out"
echo "$out" | grep -q '^SKIP:' || fail "expected SKIP"

echo "--- run 3 (expect REFUSED: +17.6% jump on a later day)"
sed -e "s/time='2026-11-13'/time='2026-11-14'/" -e "s/rate='1.1052'/rate='1.3000'/" \
  "$ROOT/services/feeder/tests/fixtures/ecb_eurofxref_daily_20261115.xml" > "$WORK/jump.xml"
set +e; out=$(feeder file:///tmpdata/jump.xml 2>&1); code=$?; set -e; echo "$out"
[ "$code" = 2 ] || fail "expected exit 2 (gate), got $code"
cli contract invoke --id $ORACLE --source admin --send=no -- get_source --source_id ECB_EURC 2>/dev/null | tail -1 | grep -q '"price":"11052000"' || fail "price changed after refusal"

echo "--- admin cannot push a price (O7a)"
if cli contract invoke --id $ORACLE --source admin -- update_price --source_id ECB_EURC --price 99000000 --liquidity_depth_usd 0 --timestamp 1794748600 >/dev/null 2>&1; then
  fail "admin was able to write a price"
fi
echo "E2E PASS"
