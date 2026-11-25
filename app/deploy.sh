#!/usr/bin/env bash
# Build the app with the testnet config (no secrets) and deploy it to the
# Vercel project "constella-app" from a durable directory outside the repo.
set -euo pipefail
cd "$(dirname "$0")"
npm run -s build
D="$HOME/.cache/constella-app-deploy"
mkdir -p "$D"
find "$D" -mindepth 1 -maxdepth 1 ! -name .vercel -exec rm -rf {} +
cp -r dist/* "$D/" && cp vercel.json "$D/"
grep -q '"LOCAL_ONLY_secrets"' "$D/config.json" && { echo "refusing: config has secrets"; exit 1; }
cd "$D"
[ -d .vercel ] || vercel link --yes --project constella-app >/dev/null
vercel deploy --prod --yes
rm -f .env.local   # vercel link writes an OIDC token here
