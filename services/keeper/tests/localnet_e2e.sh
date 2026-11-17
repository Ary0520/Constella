#!/usr/bin/env bash
# Keeper end-to-end on a REAL local Stellar network (stellar/quickstart --local)
# with Constella's release WASMs and the real mainnet Soroswap WASMs:
#   1. a user deposits 10,000 USDC into a Cash Reserve vault (85 % USDC
#      'numeraire' / 15 % EURC) -> the keeper rebalances through Soroswap,
#      validated by the signed policy, and the vault lands in its bands;
#   2. a second run does nothing (in band);
#   3. a whale moves the pool off the oracle price, new deposit -> the keeper
#      refuses to trade (nothing sent);
#   4. a key that is not the vault's keeper does nothing;
#   5. a frozen EURC price -> a user redeems (cash now + a claim); price back
#      -> the keeper settles the claim to the user;
#   6. the keeper renews idle storage (rent) for every contract, code, policy record;
#   7. a paused vault is skipped;
#   8. the watchdog emails ALERTs (paused vault, low keeper balance) to a local SMTP sink, then RESOLVED.
# Usage (repo root, after `stellar contract build`): services/keeper/tests/localnet_e2e.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
NET=constella-keeper-net
NODE=constella-keeper-node
CLI_IMAGE=constella-stellar-build:30.0.1
QS_IMAGE=${QS_IMAGE:-stellar/quickstart:latest}
PY_IMAGE=python:3.12-slim
RPC=http://$NODE:8000/soroban/rpc
PASS="Standalone Network ; February 2017"
W=$ROOT/contracts/target/wasm32v1-none/release
SW=$ROOT/contracts/vault-strategy/testdata
WORK=$(mktemp -d)
cleanup() { docker rm -f $NODE >/dev/null 2>&1 || true; docker network rm $NET >/dev/null 2>&1 || true; rm -rf "$WORK"; }
trap cleanup EXIT
fail() { echo "E2E FAIL: $*" >&2; exit 1; }
for c in registry nav_oracle policy vault_strategy; do [ -f "$W/constella_$c.wasm" ] || fail "build the contracts first"; done

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
cli() { local pre=() post=() seen=0; for a in "$@"; do if [ $seen = 0 ] && [ "$a" = "--" ]; then seen=1; fi
          if [ $seen = 1 ]; then post+=("$a"); else pre+=("$a"); fi; done
        docker run --rm --user "$U" --network $NET -v "$WORK:/work" -v "$W:/wasm:ro" -v "$SW:/swasm:ro" -e STELLAR_CONFIG_HOME=/work -e HOME=/work \
          $CLI_IMAGE stellar "${pre[@]}" --rpc-url $RPC --network-passphrase "$PASS" "${post[@]}"; }
keys() { docker run --rm --user "$U" --network $NET -v "$WORK:/work" -e STELLAR_CONFIG_HOME=/work -e HOME=/work $CLI_IMAGE stellar keys "$@"; }
fund() { for i in $(seq 1 30); do
    code=$(docker run --rm --network $NET curlimages/curl:8.10.1 -s -o /dev/null -w "%{http_code}" "http://$NODE:8000/friendbot?addr=$1")
    [ "$code" = 200 ] && return 0; sleep 5; done; fail "friendbot could not fund $1"; }
deploy_wasm() { local wasm=$1; shift; local id
  id=$(cli contract deploy --wasm "$wasm" --source admin "$@" 2>"$WORK/err" | tail -1)
  [[ $id == C* ]] || fail "deploy $wasm: $(tail -3 "$WORK/err")"; echo "$id"; }
invoke() { cli contract invoke --source "$@" >/dev/null 2>"$WORK/err" || fail "invoke $*: $(tail -3 "$WORK/err")"; }
view() { cli contract invoke --source admin --send=no "$@" 2>/dev/null | tail -1; }

for k in admin issuer feeder1 feeder2 keeper lp user stranger; do keys generate $k >/dev/null 2>&1; fund "$(keys address $k)"; done
ADMIN=$(keys address admin); ISSUER=$(keys address issuer); F1=$(keys address feeder1); F2=$(keys address feeder2)
KEEPER=$(keys address keeper); LP=$(keys address lp); USER=$(keys address user)

USDC=$(cli contract asset deploy --asset USDC:$ISSUER --source admin 2>"$WORK/err" | tail -1); [[ $USDC == C* ]] || fail "USDC: $(tail -3 "$WORK/err")"
EURC=$(cli contract asset deploy --asset EURC:$ISSUER --source admin 2>"$WORK/err" | tail -1); [[ $EURC == C* ]] || fail "EURC: $(tail -3 "$WORK/err")"

REGISTRY=$(deploy_wasm /wasm/constella_registry.wasm -- --admin $ADMIN)
reg() { invoke admin --id $REGISTRY -- add_asset --entry "{\"contract_address\":\"$1\",\"asset_id\":\"$2\",\"issuer\":\"CIRCLE\",\"sleeve_type\":0,\"liquidity_tier\":0,\"permissionless\":true,\"oracle_sources\":$3,\"eligible_jurisdictions\":[],\"blocked_jurisdictions\":[],\"investor_classes_required\":[],\"is_active\":true,\"added_at\":0}"; }
reg $USDC USDC '["USDC_BASE"]'
reg $EURC EURC '["EURC_A","EURC_B"]'

ORACLE=$(deploy_wasm /wasm/constella_nav_oracle.wasm -- --admin $ADMIN --base_asset $USDC --decimals 7 --resolution 300)
invoke admin --id $ORACLE -- configure_sleeve --sleeve_id CASH --deviation_threshold_bps 200 --min_valid_sources 2
src() { invoke admin --id $ORACLE -- register_source --source "{\"source_id\":\"$1\",\"asset\":\"$EURC\",\"sleeve\":\"CASH\",\"price\":\"0\",\"timestamp\":0,\"staleness_limit_seconds\":432000,\"min_liquidity_depth_usd\":\"0\",\"current_liquidity_depth_usd\":\"0\",\"is_active\":true,\"feeder\":\"$2\"}"; }
src EURC_A $F1; src EURC_B $F2
NOW=$(( $(date +%s) - 30 ))
invoke feeder1 --id $ORACLE -- update_price --source_id EURC_A --price 10800000 --liquidity_depth_usd 0 --timestamp $NOW
invoke feeder2 --id $ORACLE -- update_price --source_id EURC_B --price 10800000 --liquidity_depth_usd 0 --timestamp $NOW

POLICY=$(deploy_wasm /wasm/constella_policy.wasm -- --admin $ADMIN --oracle $ORACLE)
VAULT=$(deploy_wasm /wasm/constella_vault_strategy.wasm -- --config "{\"admin\":\"$ADMIN\",\"pause_authority\":\"$ADMIN\",\"numeraire\":\"$USDC\",\"policy_contract\":\"$POLICY\",\"oracle_contract\":\"$ORACLE\",\"registry_contract\":\"$REGISTRY\",\"treasury\":\"$ADMIN\",\"performance_fee_bps\":1000,\"tvl_cap\":\"10000000000000\",\"per_user_cap\":\"1000000000000\"}" --name "Constella Cash Reserve" --symbol CCR --decimals 7)

# Real Soroswap (mainnet WASMs): pair code, factory, router, liquidity at the oracle price.
PAIR_HASH=$(cli contract upload --wasm /swasm/soroswap_pair_mainnet.wasm --source admin 2>"$WORK/err" | tail -1); [ ${#PAIR_HASH} = 64 ] || fail "pair upload: $(tail -3 "$WORK/err")"
FACTORY=$(deploy_wasm /swasm/soroswap_factory_mainnet.wasm)
invoke admin --id $FACTORY -- initialize --setter $ADMIN --pair_wasm_hash $PAIR_HASH
ROUTER=$(deploy_wasm /swasm/soroswap_router_mainnet.wasm)
invoke admin --id $ROUTER -- initialize --factory $FACTORY
trust() { cli tx new change-trust --source "$1" --line "$2:$ISSUER" >/dev/null 2>"$WORK/err" || fail "trustline $1 $2: $(tail -3 "$WORK/err")"; }
trust lp USDC; trust lp EURC; trust user USDC
invoke issuer --id $USDC -- mint --to $LP --amount 20000000000000
invoke issuer --id $EURC -- mint --to $LP --amount 10000000000000
invoke lp --id $ROUTER -- add_liquidity --token_a $USDC --token_b $EURC --amount_a_desired 10800000000000 --amount_b_desired 10000000000000 --amount_a_min 0 --amount_b_min 0 --to $LP --deadline 4102444800

# Cash Reserve policy (D-A): USDC = vault currency ('numeraire'), 85 / 15, real caps.
invoke admin --id $VAULT -- bind_policy --owner $ADMIN --policy_hash $(printf 'c%.0s' $(seq 1 64)) --min_liquidity_sleeve_bps 1000 --max_single_asset_bps 2500 --max_issuer_bps 3000 --max_slippage_bps 50 --rebalance_band_bps 200 \
  --sleeves '[{"asset_id":"USDC","target_weight_bps":8500,"min_band_bps":8000,"max_band_bps":9000,"sleeve_type":"numeraire"},{"asset_id":"EURC","target_weight_bps":1500,"min_band_bps":1000,"max_band_bps":2000,"sleeve_type":"cash"}]'
invoke admin --id $VAULT -- set_sleeves --sleeves "[{\"asset_id\":\"USDC\",\"contract_address\":\"$USDC\",\"target_weight_bps\":8500,\"band_bps\":200,\"current_balance\":\"0\",\"sleeve_type\":0,\"liquidity_tier\":0},{\"asset_id\":\"EURC\",\"contract_address\":\"$EURC\",\"target_weight_bps\":1500,\"band_bps\":200,\"current_balance\":\"0\",\"sleeve_type\":2,\"liquidity_tier\":0}]"
invoke admin --id $VAULT -- set_keeper --keeper $KEEPER
invoke admin --id $VAULT -- propose_router --route 1 --router $ROUTER   # fresh vault: immediate (D-K)
[ "$(view --id $VAULT -- get_router --route 1)" = "\"$ROUTER\"" ] || fail "router not set immediately"
echo "vault $VAULT  policy $POLICY  oracle $ORACLE  router $ROUTER"

invoke issuer --id $USDC -- mint --to $USER --amount 200000000000
invoke user --id $VAULT -- deposit --assets 100000000000 --receiver $USER --from $USER --operator $USER

keeper() { docker run --rm --network $NET -v "$ROOT/services/keeper:/svc" -v "$WORK:/work" -w /svc \
  -e CONSTELLA_RPC_URL=$RPC -e "CONSTELLA_NETWORK_PASSPHRASE=$PASS" -e KEEPER_SECRET="$1" -e KEEPER_VAULTS=$VAULT \
  -e KEEPER_ONCE=1 -e KEEPER_STATE_FILE=/work/keeper_state.json $PY_IMAGE \
  sh -c 'pip install -q --no-cache-dir --root-user-action=ignore -e . >/dev/null 2>&1; constella-keeper' 2>&1 | grep '"vault"'; }
eurc_held() { view --id $VAULT -- get_sleeves | python3 -c "import json,sys; print([s for s in json.load(sys.stdin) if s['asset_id']=='EURC'][0]['current_balance'])"; }
KS=$(keys show keeper)

echo "--- 1. fresh deposit -> rebalance"; out=$(keeper "$KS"); echo "$out"
echo "$out" | grep -q '"action": "rebalanced"' || fail "expected a rebalance"
E=$(eurc_held); echo "EURC held: $E"
python3 -c "e=int('$E'.strip('\"')); v=e*1.08/(e*1.08+(100000000000-1500*10**7)); assert 0.13<=v<=0.17, v; print('EURC weight %.4f' % v)" || fail "EURC weight off target"

echo "--- 2. in band -> nothing"; out=$(keeper "$KS"); echo "$out"
echo "$out" | grep -q '"action": "none"' || fail "expected no action"

echo "--- 3. pool pushed off the oracle, new deposit -> refused"
invoke lp --id $ROUTER -- swap_exact_tokens_for_tokens --amount_in 3000000000000 --amount_out_min 0 --path "[\"$USDC\",\"$EURC\"]" --to $LP --deadline 4102444800
invoke user --id $VAULT -- deposit --assets 100000000000 --receiver $USER --from $USER --operator $USER
before=$(eurc_held); out=$(keeper "$KS"); echo "$out"
echo "$out" | grep -q '"action": "refused"' || fail "expected a refusal"
echo "$out" | grep -q 'below the oracle minimum' || fail "refusal reason"
[ "$(eurc_held)" = "$before" ] || fail "holdings changed after a refusal"

echo "--- 4. not the vault's keeper -> skip"; out=$(keeper "$(keys show stranger)"); echo "$out"
echo "$out" | grep -q "not the vault's keeper" || fail "stranger was not skipped"

echo "--- 5. frozen sleeve claim -> settled by the keeper"
invoke admin --id $ORACLE -- trigger_circuit_breaker --caller $ADMIN --sleeve_id CASH --reason E2E
SHARES=$(view --id $VAULT -- balance --id $USER | tr -d '"'); HALF=$((SHARES / 2))
invoke user --id $VAULT -- redeem_frozen --shares $HALF --receiver $USER --owner $USER --operator $USER
OWED=$(view --id $VAULT -- get_claim --holder $USER --asset $EURC | tr -d '"'); echo "claim recorded: $OWED EURC units"
[ "$OWED" -gt 0 ] || fail "no claim recorded"
invoke admin --id $ORACLE -- clear_circuit_breaker --sleeve_id CASH
usdc_before=$(view --id $USDC -- balance --id $USER | tr -d '"')
out=$(keeper "$KS"); echo "$out"
echo "$out" | grep -q '"action": "settled"' || fail "claim not settled"
[ "$(view --id $VAULT -- get_claim --holder $USER --asset $EURC | tr -d '"')" = "0" ] || fail "claim still open"
usdc_after=$(view --id $USDC -- balance --id $USER | tr -d '"')
[ "$usdc_after" -gt "$usdc_before" ] || fail "holder not paid"
echo "holder paid $((usdc_after - usdc_before)) USDC units for the claim"

echo "--- 6. storage rent: the keeper renews idle entries"
out=$(docker run --rm --network $NET -v "$ROOT/services/keeper:/svc" -v "$WORK:/work" -w /svc \
  -e CONSTELLA_RPC_URL=$RPC -e "CONSTELLA_NETWORK_PASSPHRASE=$PASS" -e KEEPER_SECRET="$KS" -e KEEPER_VAULTS=$VAULT \
  -e KEEPER_ONCE=1 -e KEEPER_STATE_FILE=/work/keeper_state.json -e KEEPER_TTL_MIN_DAYS=35 -e KEEPER_TTL_EXTEND_DAYS=40 $PY_IMAGE \
  sh -c 'pip install -q --no-cache-dir --root-user-action=ignore -e . >/dev/null 2>&1; constella-keeper' 2>&1 | grep '"rent"'); echo "$out"
echo "$out" | grep -q '"hash"' || fail "rent extension not submitted"
python3 -c "import json,sys; d=json.loads(sys.argv[1]); assert d['due']>=10 and d['checked']>=10, d" "$out" || fail "too few entries renewed"
py_ttl=$(docker run --rm --network $NET -v "$ROOT/services/keeper:/svc" -w /svc -e RPC=$RPC -e POLICY=$POLICY -e VAULT=$VAULT $PY_IMAGE sh -c '
  pip install -q --no-cache-dir --root-user-action=ignore -e . >/dev/null 2>&1; python -c "
import os; from stellar_sdk import SorobanServer
from constella_keeper.chain import data_key, enum_key, instance_key, scalar_addr
s=SorobanServer(os.environ[\"RPC\"]); latest=s.get_latest_ledger().sequence
ks=[instance_key(os.environ[\"POLICY\"]), data_key(os.environ[\"POLICY\"], enum_key(\"PolicyConfig\", scalar_addr(os.environ[\"VAULT\"])))]
print(min((e.live_until_ledger-latest)//17280 for e in s.get_ledger_entries(ks).entries))"')
echo "policy instance + PolicyConfig now live for >= $py_ttl days"
[ "$py_ttl" -ge 39 ] || fail "entries not renewed to 40 days"

echo "--- 7. paused -> skip"; invoke admin --id $VAULT -- pause; out=$(keeper "$KS"); echo "$out"
echo "$out" | grep -q '"reason": "vault paused"' || fail "paused vault not skipped"
echo "--- 8. watchdog (D-M): alert emails to a local SMTP sink, then RESOLVED"
SINK=constella-smtp-sink
docker rm -f $SINK >/dev/null 2>&1 || true
docker run -d --rm --name $SINK --network $NET $PY_IMAGE sh -c 'pip install -q --no-cache-dir --root-user-action=ignore aiosmtpd==1.4.6 >/dev/null 2>&1; python -u -m aiosmtpd -n -l 0.0.0.0:1025 -c aiosmtpd.handlers.Debugging' >/dev/null
for i in $(seq 1 60); do docker logs $SINK 2>&1 | grep -q . && break; docker run --rm --network $NET $PY_IMAGE python -c "import socket; socket.create_connection(('$SINK',1025),2)" >/dev/null 2>&1 && break; sleep 2; done
watchdog() { docker run --rm --network $NET -v "$ROOT/services/watchdog:/svc" -v "$WORK:/work" -w /svc \
  -e CONSTELLA_RPC_URL=$RPC -e "CONSTELLA_NETWORK_PASSPHRASE=$PASS" -e WATCHDOG_NETWORK=local -e WATCHDOG_SOURCE=$ADMIN \
  -e WATCHDOG_VAULTS=$VAULT -e "WATCHDOG_ACCOUNTS=keeper:$KEEPER:100000" -e WATCHDOG_ONCE=1 \
  -e WATCHDOG_STATE_FILE=/work/wd_state.json -e WATCHDOG_STATUS_FILE=/work/wd_status.json \
  -e ALERT_SMTP_HOST=$SINK -e ALERT_SMTP_PORT=1025 -e ALERT_SMTP_TLS=none -e ALERT_FROM=watchdog@constella.local -e ALERT_TO=ops@constella.local \
  $PY_IMAGE sh -c 'pip install -q --no-cache-dir --root-user-action=ignore -e . >/dev/null 2>&1; constella-watchdog' 2>&1 | grep '"action"'; }
out=$(watchdog); echo "$out"
echo "$out" | grep -q '"failing": 2' || fail "expected exactly 2 failing checks (paused vault, low keeper balance)"
sleep 2; mails=$(docker logs $SINK 2>&1)
echo "$mails" | grep -q "Subject: \[Constella local\] ALERT: vault .* is PAUSED" || fail "no paused-vault alert email"
echo "$mails" | grep -q "Subject: \[Constella local\] ALERT: keeper balance" || fail "no low-balance alert email"
invoke admin --id $VAULT -- unpause
out=$(watchdog); echo "$out"
sleep 2; docker logs $SINK 2>&1 | grep -q "Subject: \[Constella local\] RESOLVED: vault .* is running" || fail "no RESOLVED email"
out=$(watchdog); echo "$out" | grep -q '"notices": 0' || fail "a healthy re-check must not email again"
docker rm -f $SINK >/dev/null 2>&1 || true
echo "WATCHDOG PASS"
echo "KEEPER E2E PASS"
