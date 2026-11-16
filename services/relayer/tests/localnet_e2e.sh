#!/usr/bin/env bash
# Audit S2 / F1 - zero-gas proof on a REAL local Stellar network
# (stellar/quickstart --local, same protocol as testnet) with Constella's
# release WASMs: a passkey smart account holding NO XLM deposits USDC into the
# vault through the sponsor relayer's HTTP API; real fees are charged to the
# relayer, quotas are recorded on the fee-sponsor contract, and abusive
# requests are refused before anything is spent.
# Usage (repo root, after `stellar contract build`): services/relayer/tests/localnet_e2e.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
NET=constella-relayer-net
NODE=constella-relayer-node
RELAY=constella-relayer-svc
CLI_IMAGE=constella-stellar-build:30.0.1
QS_IMAGE=${QS_IMAGE:-stellar/quickstart:latest}
PY_IMAGE=python:3.12-slim
RPC=http://$NODE:8000/soroban/rpc
PASS="Standalone Network ; February 2017"
W=$ROOT/contracts/target/wasm32v1-none/release
WORK=$(mktemp -d)
cleanup() { docker rm -f $NODE $RELAY >/dev/null 2>&1 || true; docker network rm $NET >/dev/null 2>&1 || true; rm -rf "$WORK"; }
trap cleanup EXIT
fail() { echo "E2E FAIL: $*" >&2; [ -n "${RELAY_LOGS:-}" ] && docker logs $RELAY 2>&1 | tail -20 >&2; exit 1; }
for c in registry nav_oracle policy vault_strategy webauthn_verifier smart_account fee_sponsor; do
  [ -f "$W/constella_$c.wasm" ] || fail "build the contracts first ($W/constella_$c.wasm missing)"
done

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
        docker run --rm --user "$U" --network $NET -v "$WORK:/work" -v "$W:/wasm:ro" -e STELLAR_CONFIG_HOME=/work -e HOME=/work \
          $CLI_IMAGE stellar "${pre[@]}" --rpc-url $RPC --network-passphrase "$PASS" "${post[@]}"; }
keys() { docker run --rm --user "$U" --network $NET -v "$WORK:/work" -e STELLAR_CONFIG_HOME=/work -e HOME=/work $CLI_IMAGE stellar keys "$@"; }
fund() {
  for i in $(seq 1 30); do
    code=$(docker run --rm --network $NET curlimages/curl:8.10.1 -s -o /dev/null -w "%{http_code}" "http://$NODE:8000/friendbot?addr=$1")
    [ "$code" = 200 ] && return 0; sleep 5
  done
  fail "friendbot could not fund $1"
}
deploy() { local name=$1; shift; local id
  id=$(cli contract deploy --wasm /wasm/constella_$name.wasm --source admin "$@" 2>"$WORK/err" | tail -1)
  [[ $id == C* ]] || fail "$name deploy: $(tail -3 "$WORK/err")"; echo "$id"; }
invoke() { cli contract invoke --source "$@" >/dev/null 2>"$WORK/err" || fail "invoke $*: $(tail -3 "$WORK/err")"; }

for k in admin issuer relayer viewer; do keys generate $k >/dev/null 2>&1; fund "$(keys address $k)"; done
ADMIN=$(keys address admin); ISSUER=$(keys address issuer); RELAYER=$(keys address relayer); VIEWER=$(keys address viewer)
RELAYER_SECRET=$(keys show relayer)

USDC=$(cli contract asset deploy --asset USDC:$ISSUER --source admin 2>"$WORK/err" | tail -1); [[ $USDC == C* ]] || fail "USDC: $(tail -3 "$WORK/err")"
cli contract asset deploy --asset native --source admin >/dev/null 2>&1 || true
NATIVE=$(cli contract id asset --asset native 2>/dev/null | tail -1); [[ $NATIVE == C* ]] || fail "native SAC id"

REGISTRY=$(deploy registry -- --admin $ADMIN)
invoke admin --id $REGISTRY -- add_asset --entry "{\"contract_address\":\"$USDC\",\"asset_id\":\"USDC\",\"issuer\":\"CIRCLE\",\"sleeve_type\":0,\"liquidity_tier\":0,\"permissionless\":true,\"oracle_sources\":[\"USDC_BASE\"],\"eligible_jurisdictions\":[],\"blocked_jurisdictions\":[],\"investor_classes_required\":[],\"is_active\":true,\"added_at\":0}"
ORACLE=$(deploy nav_oracle -- --admin $ADMIN --base_asset $USDC --decimals 7 --resolution 300)
POLICY=$(deploy policy -- --admin $ADMIN --oracle $ORACLE)
VAULT=$(deploy vault_strategy -- --config "{\"admin\":\"$ADMIN\",\"pause_authority\":\"$ADMIN\",\"numeraire\":\"$USDC\",\"policy_contract\":\"$POLICY\",\"oracle_contract\":\"$ORACLE\",\"registry_contract\":\"$REGISTRY\",\"treasury\":\"$ADMIN\",\"management_fee_bps\":0,\"tvl_cap\":\"10000000000000\",\"per_user_cap\":\"1000000000000\"}" --name "Constella Vault" --symbol CONST --decimals 7)
VERIFIER=$(deploy webauthn_verifier)
SPONSOR=$(deploy fee_sponsor -- --admin $ADMIN --max_ops_per_hour 10 --max_ops_per_day 50 --daily_budget_stroops 100000000)
invoke admin --id $SPONSOR -- whitelist_contract --target $VAULT
invoke admin --id $SPONSOR -- set_relayer --relayer $RELAYER

py() { docker run --rm --network $NET -v "$ROOT/services/relayer:/svc" -v "$WORK:/work" -w /svc "$@"; }
PUBKEY=$(py $PY_IMAGE sh -c 'pip install -q --no-cache-dir --root-user-action=ignore -e ".[dev]" >/dev/null 2>&1; python tests/localnet_driver.py keygen /work/passkey.pem')
[ ${#PUBKEY} = 130 ] || fail "passkey keygen: $PUBKEY"
ACCOUNT=$(deploy smart_account -- --verifier $VERIFIER --owner_key $PUBKEY --name owner)
invoke issuer --id $USDC -- mint --to $ACCOUNT --amount 10000000000
echo "registry $REGISTRY  vault $VAULT  sponsor $SPONSOR  account $ACCOUNT  relayer $RELAYER"

docker run -d --rm --name $RELAY --network $NET -v "$ROOT/services/relayer:/svc" -w /svc \
  -e CONSTELLA_RPC_URL=$RPC -e "CONSTELLA_NETWORK_PASSPHRASE=$PASS" -e RELAYER_SECRET="$RELAYER_SECRET" \
  -e SPONSOR_CONTRACT_ID=$SPONSOR -e "SPONSOR_ALLOWED=$VAULT:deposit,redeem,withdraw,redeem_frozen" \
  -e RELAYER_HOST=0.0.0.0 -e RELAYER_PORT=8787 $PY_IMAGE \
  sh -c 'pip install -q --no-cache-dir --root-user-action=ignore -e . >/dev/null 2>&1; constella-relayer' >/dev/null
RELAY_LOGS=1
for i in $(seq 1 60); do
  docker run --rm --network $NET curlimages/curl:8.10.1 -s http://$RELAY:8787/health 2>/dev/null | grep -q '"ok": true' && break
  sleep 2; [ "$i" = 60 ] && fail "relayer never became healthy"
done

drive() { py -e RPC=$RPC -e "PASS=$PASS" -e VIEWER=$VIEWER -e RELAYER_URL=http://$RELAY:8787 -e ACCOUNT=$ACCOUNT \
  -e VAULT=$VAULT -e USDC=$USDC -e NATIVE=$NATIVE -e RELAYER=$RELAYER -e SPONSOR=$SPONSOR -e VERIFIER=$VERIFIER \
  -e PASSKEY_PEM=/work/passkey.pem $PY_IMAGE sh -c "pip install -q --no-cache-dir --root-user-action=ignore -e '.[dev]' >/dev/null 2>&1; python tests/localnet_driver.py $1"; }
drive run || fail "zero-gas run"
invoke admin --id $SPONSOR -- pause
drive paused || fail "paused run"
echo "E2E PASS"
