#!/usr/bin/env node
/**
 * BitLab Coin Presale — operator CLI.
 *
 * This is NOT the public website. It is the only supported way to
 * initialize / freeze / start / pause / end / finalize the Smart Contract
 * without hand-building instruction bytes.
 *
 * Usage (WSL, from this folder after `npm install`):
 *   node scripts/operator.mjs --help
 *   node scripts/operator.mjs --keypair /mnt/c/Users/Techc/treasury.json status
 */

import { createHash } from "crypto";
import fs from "fs";
import os from "os";
import path from "path";
import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
  sendAndConfirmTransaction,
} from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  createAssociatedTokenAccountIdempotentInstruction,
  createTransferCheckedInstruction,
  getAccount,
  getAssociatedTokenAddressSync,
  getMint,
} from "@solana/spl-token";

const PROGRAM_ID = new PublicKey("Bit7n73gYrnXMPXeeA3yiuoUsLRZ5ztXqzCZrduLaT5n");
const BITLAB_MINT = new PublicKey("CCVYpFakBd4kkwn7G4rvbpaiB5pGJwUpiNwNYrkC5ygQ");
const USDC_MINT = new PublicKey("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
const USDT_MINT = new PublicKey("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB");
const PYTH_SOL_USD = new PublicKey("H6ARHf6YXhGYeQfUzQNGk6rDNnLBQKrenN712K4AQJEG");
const TREASURY = new PublicKey("3qFihzuw6BdMVC4HbQ4owHsEYGRs2pXE3huxzChuEMDm");
const STATE_SEED = Buffer.from("bitlab-presale");
const SOL_VAULT_SEED = Buffer.from("bitlab-sol-vault");
const ALLOCATION_UI = 7_500_000_000;
const BITLAB_DECIMALS = 6;
const DEFAULT_MIN = 10_000_000n;
const DEFAULT_MAX = 100_000_000_000n;
const DEFAULT_WALLET_CAP = 500_000_000_000n;
const DEFAULT_STALE = 60n;

function disc(name) {
  return createHash("sha256").update(`global:${name}`).digest().subarray(0, 8);
}

function parseArgs(argv) {
  const out = { _: [], flags: {} };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith("--")) {
      const key = a.slice(2);
      const next = argv[i + 1];
      if (!next || next.startsWith("--")) out.flags[key] = true;
      else {
        out.flags[key] = next;
        i++;
      }
    } else out._.push(a);
  }
  return out;
}

function loadKeypair(filePath) {
  const raw = fs.readFileSync(filePath, "utf8");
  const arr = JSON.parse(raw);
  if (!Array.isArray(arr) || arr.length < 64) {
    throw new Error(
      `Keypair file ${filePath} is not a Solana JSON keypair (expected an array of 64 numbers).`,
    );
  }
  return Keypair.fromSecretKey(Uint8Array.from(arr.slice(0, 64)));
}

function u64le(n) {
  const b = Buffer.alloc(8);
  b.writeBigUInt64LE(BigInt(n));
  return b;
}
function i64le(n) {
  const b = Buffer.alloc(8);
  b.writeBigInt64LE(BigInt(n));
  return b;
}

function pda(seeds) {
  return PublicKey.findProgramAddressSync(seeds, PROGRAM_ID);
}

function ata(mint, owner) {
  return getAssociatedTokenAddressSync(mint, owner, true, TOKEN_PROGRAM_ID);
}

function help() {
  console.log(`
BitLab Coin Presale operator
Program ID: ${PROGRAM_ID.toBase58()}

Read-only (no keypair required):
  node scripts/operator.mjs status
  node scripts/operator.mjs unix --at 2026-10-01T00:00:00Z

Needs a JSON keypair for OPS only (start/pause). 3qFihzuw… is a Ledger:
it receives SOL/USDC/USDT and unsold BITLAB. Never export a Ledger key.
  node scripts/operator.mjs --keypair /home/devtool/bitlab-presale/bitlab-ops.json create-vaults
  node scripts/operator.mjs --keypair /home/devtool/bitlab-presale/bitlab-ops.json initialize --at 2026-10-01T00:00:00Z
  node scripts/operator.mjs vault   (prints the BITLAB vault to fund from Phantom Ledger)
  node scripts/operator.mjs --keypair /home/devtool/bitlab-presale/bitlab-ops.json deposit-bitlab
  node scripts/operator.mjs --keypair /home/devtool/bitlab-presale/bitlab-ops.json finalize-config
  node scripts/operator.mjs --keypair /home/devtool/bitlab-presale/bitlab-ops.json start-sale --at 2026-10-01T00:00:00Z
  node scripts/operator.mjs --keypair /home/devtool/bitlab-presale/bitlab-ops.json start-sale --now
  node scripts/operator.mjs --keypair /home/devtool/bitlab-presale/bitlab-ops.json pause
  node scripts/operator.mjs --keypair /home/devtool/bitlab-presale/bitlab-ops.json resume
  node scripts/operator.mjs --keypair /home/devtool/bitlab-presale/bitlab-ops.json end
  node scripts/operator.mjs --keypair /home/devtool/bitlab-presale/bitlab-ops.json emergency-stop
  node scripts/operator.mjs --keypair /home/devtool/bitlab-presale/bitlab-ops.json clear-emergency
  node scripts/operator.mjs --keypair /home/devtool/bitlab-presale/bitlab-ops.json finalize-sale


Flags:
  --keypair FILE   Solana JSON keypair (64-byte array). Default: ~/.config/solana/id.json
  --rpc URL        Default: https://api.mainnet-beta.solana.com
  --dry-run        Build the transaction and print it. Do not send.
  --at ISO         Timestamp like 2026-10-01T00:00:00Z (UTC).
  --now            Use the current time (only for start-sale / initialize).
  --yes            Required on initialize and deposit-bitlab (they move real funds / are one-shot).

STOP if any printed address is not the one listed in this help.
`);
}

function parseWhen(flags) {
  if (flags.now) return BigInt(Math.floor(Date.now() / 1000));
  if (flags.at) {
    const ms = Date.parse(String(flags.at));
    if (Number.isNaN(ms)) throw new Error(`Could not parse --at ${flags.at}. Use 2026-10-01T00:00:00Z`);
    return BigInt(Math.floor(ms / 1000));
  }
  if (flags.unix) return BigInt(flags.unix);
  throw new Error("Pass --at 2026-10-01T00:00:00Z or --now (or --unix SECONDS).");
}

async function main() {
  const { _, flags } = parseArgs(process.argv.slice(2));
  const cmd = _[0] || "help";
  if (cmd === "help" || flags.help) {
    help();
    return;
  }

  const rpc = String(flags.rpc || process.env.SOLANA_RPC || "https://api.mainnet-beta.solana.com");
  const connection = new Connection(rpc, "confirmed");
  const [statePda] = pda([STATE_SEED]);
  const [solVaultPda] = pda([SOL_VAULT_SEED]);
  const bitlabVault = ata(BITLAB_MINT, statePda);
  const usdcVault = ata(USDC_MINT, statePda);
  const usdtVault = ata(USDT_MINT, statePda);
  const treasuryUsdc = ata(USDC_MINT, TREASURY);
  const treasuryUsdt = ata(USDT_MINT, TREASURY);
  const treasuryBitlab = ata(BITLAB_MINT, TREASURY);

  if (cmd === "unix") {
    const ts = parseWhen(flags);
    console.log(`unix: ${ts}`);
    console.log(`utc:  ${new Date(Number(ts) * 1000).toISOString()}`);
    return;
  }

  if (cmd === "status" || cmd === "vault") {
    console.log("RPC              ", rpc);
    console.log("Program          ", PROGRAM_ID.toBase58());
    console.log("State PDA        ", statePda.toBase58());
    console.log("SOL vault PDA    ", solVaultPda.toBase58());
    console.log("BITLAB vault ATA ", bitlabVault.toBase58());
    console.log("USDC vault ATA   ", usdcVault.toBase58());
    console.log("USDT vault ATA   ", usdtVault.toBase58());
    console.log("Treasury         ", TREASURY.toBase58());
    console.log("Treasury USDC    ", treasuryUsdc.toBase58());
    console.log("Treasury USDT    ", treasuryUsdt.toBase58());
    console.log("Treasury BITLAB  ", treasuryBitlab.toBase58());
    console.log("Pyth SOL/USD     ", PYTH_SOL_USD.toBase58());

    const prog = await connection.getAccountInfo(PROGRAM_ID);
    if (!prog) {
      console.log("\nProgram account: MISSING. The Smart Contract is not deployed yet. Stop here and do the buffer deploy first.");
      return;
    }
    console.log("\nProgram account: present");
    console.log("  executable     ", prog.executable);
    console.log("  owner          ", prog.owner.toBase58());
    console.log("  data bytes     ", prog.data.length);

    const st = await connection.getAccountInfo(statePda);
    if (!st) {
      console.log("\nState PDA: empty. initialize has not been run.");
    } else {
      console.log("\nState PDA: present");
      console.log("  owner          ", st.owner.toBase58());
      console.log("  data bytes     ", st.data.length);
      if (!st.owner.equals(PROGRAM_ID)) {
        console.log("  STOP: state is not owned by the presale program.");
      }
    }

    async function showAta(label, address, mint) {
      try {
        const acc = await getAccount(connection, address);
        const mintInfo = await getMint(connection, mint);
        const ui = Number(acc.amount) / 10 ** mintInfo.decimals;
        console.log(`\n${label}`);
        console.log("  address        ", address.toBase58());
        console.log("  owner          ", acc.owner.toBase58());
        console.log("  mint           ", acc.mint.toBase58());
        console.log("  amount (raw)   ", acc.amount.toString());
        console.log("  amount (UI)    ", ui);
      } catch {
        console.log(`\n${label}: does not exist yet (run create-vaults).`);
      }
    }
    await showAta("BITLAB vault", bitlabVault, BITLAB_MINT);
    await showAta("USDC vault", usdcVault, USDC_MINT);
    await showAta("USDT vault", usdtVault, USDT_MINT);
    await showAta("Treasury USDC", treasuryUsdc, USDC_MINT);
    await showAta("Treasury USDT", treasuryUsdt, USDT_MINT);
    await showAta("Treasury BITLAB (unsold destination)", treasuryBitlab, BITLAB_MINT);
    if (cmd === "vault") {
      console.log("\nSend 7,500,000,000 BITLAB from Phantom Ledger 3qFihzuw… to:");
      console.log(bitlabVault.toBase58());
    }
    return;
  }

  const defaultKp = path.join(os.homedir(), ".config", "solana", "id.json");
  const kpPath = String(flags.keypair || process.env.SOLANA_KEYPAIR || defaultKp);
  if (!fs.existsSync(kpPath)) {
    throw new Error(
      `Keypair file not found: ${kpPath}\nCreate one with solana-keygen, or pass --keypair /mnt/c/Users/Techc/treasury.json`,
    );
  }
  const payer = loadKeypair(kpPath);
  console.log("Signer           ", payer.publicKey.toBase58());
  console.log("Keypair file     ", kpPath);
  console.log("RPC              ", rpc);

  if (cmd !== "create-vaults" && cmd !== "deposit-bitlab") {
    console.log(
      `\nRoles:\n  ops signer (this JSON)     ${payer.publicKey.toBase58()}\n  money / unsold / withdraw  ${TREASURY.toBase58()}  (Ledger — do not export)`,
    );
  }

  async function send(ixs, label) {
    const tx = new Transaction().add(...ixs);
    tx.feePayer = payer.publicKey;
    const { blockhash, lastValidBlockHeight } = await connection.getLatestBlockhash();
    tx.recentBlockhash = blockhash;
    if (flags["dry-run"]) {
      console.log(`\nDRY RUN — not sent: ${label}`);
      console.log("instructions", ixs.length);
      return null;
    }
    console.log(`\nSending: ${label}`);
    const sig = await sendAndConfirmTransaction(connection, tx, [payer], {
      commitment: "confirmed",
    });
    console.log("Signature        ", sig);
    console.log("Solscan          ", `https://solscan.io/tx/${sig}`);
    console.log("Explorer         ", `https://explorer.solana.com/tx/${sig}`);
    void lastValidBlockHeight;
    return sig;
  }

  if (cmd === "create-vaults") {
    const ixs = [];
    const plan = [
      ["BITLAB vault (state PDA)", BITLAB_MINT, statePda, bitlabVault],
      ["USDC vault (state PDA)", USDC_MINT, statePda, usdcVault],
      ["USDT vault (state PDA)", USDT_MINT, statePda, usdtVault],
      ["Treasury USDC", USDC_MINT, TREASURY, treasuryUsdc],
      ["Treasury USDT", USDT_MINT, TREASURY, treasuryUsdt],
      ["Treasury BITLAB (unsold)", BITLAB_MINT, TREASURY, treasuryBitlab],
    ];
    for (const [label, mint, owner, address] of plan) {
      const info = await connection.getAccountInfo(address);
      if (info) {
        console.log("exists           ", label, address.toBase58());
      } else {
        console.log("will create      ", label, address.toBase58());
        ixs.push(
          createAssociatedTokenAccountIdempotentInstruction(
            payer.publicKey,
            address,
            owner,
            mint,
            TOKEN_PROGRAM_ID,
            ASSOCIATED_TOKEN_PROGRAM_ID,
          ),
        );
      }
    }
    if (!ixs.length) {
      console.log("\nAll token accounts already exist. Nothing to send.");
      return;
    }
    await send(ixs, "create associated token accounts");
    return;
  }

  if (cmd === "initialize") {
    if (!flags.yes) {
      throw new Error(
        "initialize can only run ONCE. Re-run with --yes after you have checked:\n" +
          "  1. Program is deployed at Bit7n73g…\n" +
          "  2. create-vaults succeeded\n" +
          "  3. 7.5B BITLAB is in the vault (send from Phantom Ledger to the address from: node scripts/operator.mjs vault)\n" +
          "  4. this JSON is the OPS wallet you will keep (not the Ledger, not deploy-hot)\n" +
          "  5. proceeds + unsold BITLAB go to Ledger 3qFihzuw…",
      );
    }
    const start = parseWhen(flags);
    console.log("sale_start_ts    ", start.toString(), new Date(Number(start) * 1000).toISOString());
    console.log("ops_authority    ", payer.publicKey.toBase58());
    console.log("treasury/unsold  ", TREASURY.toBase58());
    console.log("treasury_auth    ", TREASURY.toBase58());
    console.log("emergency        ", payer.publicKey.toBase58(), "(same as ops, so you can halt without Ledger)");
    const existing = await connection.getAccountInfo(statePda);
    if (existing) throw new Error("State PDA already exists. initialize already ran. Do NOT send it again.");

    const data = Buffer.concat([
      disc("initialize"),
      payer.publicKey.toBuffer(), // ops_authority = this JSON
      TREASURY.toBuffer(), // treasury_authority = Ledger (withdraw proceeds)
      payer.publicKey.toBuffer(), // emergency_authority = ops (halt without Ledger)
      TREASURY.toBuffer(), // treasury = Ledger (SOL/USDC/USDT land here)
      TREASURY.toBuffer(), // unsold_destination = Ledger
      PYTH_SOL_USD.toBuffer(),
      BITLAB_MINT.toBuffer(),
      i64le(start),
      u64le(DEFAULT_MIN),
      u64le(DEFAULT_MAX),
      u64le(DEFAULT_WALLET_CAP),
      i64le(DEFAULT_STALE),
    ]);
    const ix = new TransactionInstruction({
      programId: PROGRAM_ID,
      keys: [
        { pubkey: payer.publicKey, isSigner: true, isWritable: true },
        { pubkey: statePda, isSigner: false, isWritable: true },
        { pubkey: solVaultPda, isSigner: false, isWritable: true },
        { pubkey: BITLAB_MINT, isSigner: false, isWritable: false },
        { pubkey: USDC_MINT, isSigner: false, isWritable: false },
        { pubkey: USDT_MINT, isSigner: false, isWritable: false },
        { pubkey: bitlabVault, isSigner: false, isWritable: false },
        { pubkey: usdcVault, isSigner: false, isWritable: false },
        { pubkey: usdtVault, isSigner: false, isWritable: false },
        { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
      ],
      data,
    });
    await send([ix], "initialize");
    return;
  }

  if (cmd === "deposit-bitlab") {
    if (!flags.yes) {
      throw new Error(
        `deposit-bitlab will send ${ALLOCATION_UI.toLocaleString()} BITLAB from this wallet to the vault.\nRe-run with --yes when you are sure.`,
      );
    }
    const mint = await getMint(connection, BITLAB_MINT);
    if (mint.decimals !== BITLAB_DECIMALS) {
      throw new Error(`BITLAB decimals are ${mint.decimals}, expected ${BITLAB_DECIMALS}. STOP.`);
    }
    if (mint.mintAuthority !== null) {
      throw new Error("BITLAB mint authority is still set. Revoke it before depositing. STOP.");
    }
    if (mint.freezeAuthority !== null) {
      throw new Error("BITLAB freeze authority is still set. Revoke it before depositing. STOP.");
    }
    const src = ata(BITLAB_MINT, payer.publicKey);
    const srcAcc = await getAccount(connection, src);
    const raw = BigInt(ALLOCATION_UI) * 10n ** BigInt(BITLAB_DECIMALS);
    if (srcAcc.amount < raw) {
      throw new Error(
        `This JSON wallet does not hold 7,500,000,000 BITLAB.\n` +
          `  have raw ${srcAcc.amount}\n  need raw ${raw}\n` +
          `If BITLAB is on Ledger 3qFihzuw…, do NOT export the key.\n` +
          `In Phantom (that Ledger account) send 7,500,000,000 BITLAB to:\n  ${bitlabVault.toBase58()}`,
      );
    }
    const ix = createTransferCheckedInstruction(
      src,
      BITLAB_MINT,
      bitlabVault,
      payer.publicKey,
      raw,
      BITLAB_DECIMALS,
      [],
      TOKEN_PROGRAM_ID,
    );
    await send([ix], "deposit 7,500,000,000 BITLAB into vault");
    const after = await getAccount(connection, bitlabVault);
    const ui = Number(after.amount) / 10 ** BITLAB_DECIMALS;
    console.log("Vault BITLAB now ", ui);
    if (ui < ALLOCATION_UI) throw new Error("Vault is short. Do not finalize-config.");
    return;
  }

  function opsIx(name, extra = Buffer.alloc(0)) {
    return new TransactionInstruction({
      programId: PROGRAM_ID,
      keys: [
        { pubkey: payer.publicKey, isSigner: true, isWritable: false },
        { pubkey: statePda, isSigner: false, isWritable: true },
      ],
      data: Buffer.concat([disc(name), extra]),
    });
  }

  if (cmd === "finalize-config") {
    await send([opsIx("finalize_config")], "finalize_config");
    return;
  }
  if (cmd === "start-sale") {
    const start = parseWhen(flags);
    console.log("sale_start_ts    ", start.toString(), new Date(Number(start) * 1000).toISOString());
    await send([opsIx("start_sale", i64le(start))], "start_sale");
    return;
  }
  if (cmd === "pause") {
    await send([opsIx("pause")], "pause");
    return;
  }
  if (cmd === "resume") {
    await send([opsIx("resume")], "resume");
    return;
  }
  if (cmd === "end") {
    await send([opsIx("end_presale")], "end_presale");
    return;
  }
  if (cmd === "emergency-stop") {
    await send([opsIx("emergency_stop")], "emergency_stop");
    return;
  }
  if (cmd === "clear-emergency") {
    await send([opsIx("clear_emergency")], "clear_emergency");
    return;
  }
  if (cmd === "finalize-sale") {
    const ix = new TransactionInstruction({
      programId: PROGRAM_ID,
      keys: [
        { pubkey: payer.publicKey, isSigner: true, isWritable: false },
        { pubkey: statePda, isSigner: false, isWritable: true },
        { pubkey: bitlabVault, isSigner: false, isWritable: true },
        { pubkey: treasuryBitlab, isSigner: false, isWritable: true },
        { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
      ],
      data: disc("finalize_sale"),
    });
    await send([ix], "finalize_sale");
    return;
  }

  help();
  throw new Error(`Unknown command: ${cmd}`);
}

main().catch((err) => {
  console.error("\nFAILED");
  console.error(err instanceof Error ? err.message : err);
  process.exit(1);
});
