/**
 * Stellar wallet identity (Freighter/Varial, xBull, LOBSTR, Albedo, Hana, ...
 * via Stellar Wallets Kit). A wallet user holds a CLASSIC account (G...): it
 * pays its own XLM fees, needs a USDC trustline, and signs whole
 * transactions in the wallet - no relayer. Same vault, same shares as
 * passkey accounts.
 *
 * Everything except the wallet popup goes through `WalletSigner`, so the
 * whole path is tested with a local keypair signer (test/wallet.e2e.test.ts).
 */
import { Asset, Keypair, Operation, TransactionBuilder, xdr } from "@stellar/stellar-sdk";
import { Chain, funcB64 } from "./chain";

const X = xdr as any;

export interface WalletSigner {
  address: string;
  label: string;
  sign(txXdr: string): Promise<string>;
}

/** Test / script signer (local keys only). */
export function keypairSigner(secret: string, passphrase: string): WalletSigner {
  const kp = Keypair.fromSecret(secret);
  return {
    address: kp.publicKey(), label: "local keypair",
    async sign(txXdr) { const tx = TransactionBuilder.fromXDR(txXdr, passphrase); tx.sign(kp); return tx.toXDR(); },
  };
}

/** The real thing: Stellar Wallets Kit picker, then the wallet signs. */
export async function connectWallet(passphrase: string): Promise<WalletSigner> {
  const { StellarWalletsKit } = await import("@creit.tech/stellar-wallets-kit/sdk");
  const { defaultModules } = await import("@creit.tech/stellar-wallets-kit/modules/utils");
  StellarWalletsKit.init({ modules: defaultModules(), network: passphrase as any });
  const { address } = await StellarWalletsKit.authModal();
  return {
    address, label: "Stellar wallet",
    async sign(txXdr) {
      const { signedTxXdr } = await StellarWalletsKit.signTransaction(txXdr, { networkPassphrase: passphrase, address });
      return signedTxXdr;
    },
  };
}

export interface WalletStatus { exists: boolean; xlm: bigint; hasTrustline: boolean; usdc: bigint }

/** Read straight from the ledger (no Horizon needed by this page). */
export async function walletStatus(chain: Chain, address: string): Promise<WalletStatus> {
  const accountKey = X.LedgerKey.account(new X.LedgerKeyAccount({ accountId: Keypair.fromPublicKey(address).xdrAccountId() }));
  const usdc = new Asset("USDC", chain.cfg.assets.issuer);
  const tlKey = X.LedgerKey.trustline(new X.LedgerKeyTrustLine({ accountId: Keypair.fromPublicKey(address).xdrAccountId(), asset: usdc.toTrustLineXDRObject() }));
  const res: any = await chain.server.getLedgerEntries(accountKey, tlKey);
  let xlm = 0n, exists = false, hasTrustline = false, bal = 0n;
  for (const e of res.entries) {
    // stellar-sdk 17.4 XDR: property access (immutable objects).
    const d = e.val;
    if (d.type === "account") { exists = true; xlm = BigInt(String(d.account.balance)); }
    if (d.type === "trustline" || d.type === "trustLine") { hasTrustline = true; bal = BigInt(String((d.trustLine ?? d.trustline).balance)); }
  }
  return { exists, xlm, hasTrustline, usdc: bal };
}

async function submit(chain: Chain, signer: WalletSigner, build: (b: TransactionBuilder) => TransactionBuilder, soroban: boolean) {
  const src = await chain.server.getAccount(signer.address);
  let tx = build(new TransactionBuilder(src, { fee: "1000", networkPassphrase: chain.cfg.network.passphrase })).setTimeout(120).build();
  if (soroban) tx = await chain.server.prepareTransaction(tx); // simulation sets the resource fee
  const fee = Number(tx.fee);
  const signed = TransactionBuilder.fromXDR(await signer.sign(tx.toXDR()), chain.cfg.network.passphrase);
  const sent = await chain.server.sendTransaction(signed);
  if (sent.status !== "PENDING" && sent.status !== "DUPLICATE") throw new Error(`not accepted: ${sent.status}`);
  const res: any = await chain.server.pollTransaction(sent.hash, { attempts: 30 });
  if (res.status !== "SUCCESS") throw new Error(`failed on chain: ${res.status}`);
  return { hash: sent.hash, max_fee_xlm: fee / 1e7 };
}

/** Local / testnet only: friendbot creates and funds the account. */
export async function friendbotFund(chain: Chain, address: string) {
  const r = await fetch(`${chain.cfg.network.friendbot}?addr=${address}`);
  if (!r.ok) throw new Error(`friendbot ${r.status}`);
  return { funded: address };
}

export function addUsdcTrustline(chain: Chain, signer: WalletSigner) {
  return submit(chain, signer, (b) => b.addOperation(Operation.changeTrust({ asset: new Asset("USDC", chain.cfg.assets.issuer) })), false);
}

/** A contract call the wallet user signs and pays for (source-account auth). */
export function walletInvoke(chain: Chain, signer: WalletSigner, contract: string, fn: string, args: xdr.ScVal[]) {
  const func = X.HostFunction.fromXDR(funcB64(contract, fn, args), "base64");
  return submit(chain, signer, (b) => b.addOperation(Operation.invokeHostFunction({ func, auth: [] })), true);
}
