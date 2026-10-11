#!/usr/bin/env node
// Dump the raw Riot API payloads for the current VALORANT state (menus, agent
// select or in game) to captures/<timestamp>_<state>/, one pretty JSON file per
// endpoint plus a manifest.json of every request made or skipped. Zero
// dependencies (plain Node). Mirrors the request shapes in src-tauri/src/riot/.
//
// Usage:
//   node scripts/snapshot.mjs                    one request per endpoint
//   node scripts/snapshot.mjs --match-details    also fetch match-details
//
// The lockfile password and the access/entitlements tokens never reach disk or
// stdout. Captures still hold real puuids and player names: do not share them.

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, relative } from "node:path";
import https from "node:https";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

// Same values as src-tauri/src/riot/constants.rs.
const CLIENT_PLATFORM =
  "ew0KCSJwbGF0Zm9ybVR5cGUiOiAiUEMiLA0KCSJwbGF0Zm9ybU9TIjogIldpbmRvd3MiLA0KCSJwbGF0Zm9ybU9TVmVyc2lvbiI6ICIxMC4wLjE5MDQyLjEuMjU2LjY0Yml0IiwNCgkicGxhdGZvcm1DaGlwc2V0IjogIlVua25vd24iDQp9";
const USER_AGENT = "ShooterGame/13 Windows/10.0.19043.1.256.64bit";
const COMPETITIVE_UPDATES_END_INDEX = 10;
const VALORANT_API = "https://valorant-api.com/v1";

const LOCAL_TIMEOUT_MS = 10_000;
const REMOTE_TIMEOUT_MS = 15_000;

const withMatchDetails = process.argv.slice(2).includes("--match-details");

function fail(message) {
  console.error(`snapshot: ${message}`);
  process.exit(1);
}

// --- lockfile (lockfile.rs) ------------------------------------------------

function readLockfile() {
  const localAppData = process.env.LOCALAPPDATA;
  if (!localAppData) fail("%LOCALAPPDATA% is not set; cannot locate the Riot lockfile");
  const path = join(localAppData, "Riot Games", "Riot Client", "Config", "lockfile");
  let contents;
  try {
    contents = readFileSync(path, "utf8");
  } catch (err) {
    if (err.code === "ENOENT") {
      fail("Riot Client lockfile not found; start VALORANT and run this again");
    }
    fail(`could not read the Riot lockfile: ${err.code ?? err.message}`);
  }
  const parts = contents.trim().split(":");
  if (parts.length !== 5) {
    fail(`malformed lockfile: expected 5 colon-separated fields, got ${parts.length}`);
  }
  const port = Number(parts[2]);
  if (!Number.isInteger(port) || port <= 0 || port > 65535) fail("malformed lockfile: bad port");
  return { port, password: parts[3] };
}

// --- HTTP ------------------------------------------------------------------

// The local client serves a self-signed cert; this agent is only ever used for 127.0.0.1.
const localAgent = new https.Agent({ rejectUnauthorized: false });

function request(url, { headers, agent, timeoutMs }) {
  return new Promise((resolve) => {
    let settled = false;
    const settle = (result) => {
      if (settled) return;
      settled = true;
      resolve(result);
    };
    const failed = (err) => settle({ status: null, body: null, error: err?.message ?? "response aborted" });
    const req = https.get(url, { headers, agent, timeout: timeoutMs }, (res) => {
      res.on("error", failed);
      res.on("aborted", () => failed(new Error("response aborted")));
      const chunks = [];
      res.on("data", (chunk) => chunks.push(chunk));
      res.on("end", () => {
        const text = Buffer.concat(chunks).toString("utf8");
        let body = text;
        try {
          body = text ? JSON.parse(text) : null;
        } catch {
          // Non-JSON bodies are kept as text.
        }
        settle({ status: res.statusCode, body });
      });
    });
    req.on("timeout", () => req.destroy(new Error(`timed out after ${timeoutMs}ms`)));
    req.on("error", failed);
  });
}

// --- capture bookkeeping ---------------------------------------------------

const files = new Map(); // file name -> JSON value
const manifest = [];

/**
 * One request, recorded in the manifest whatever its outcome. A body is saved
 * to `file` only when `save` is set (never for the token endpoint). Returns the
 * parsed body on a 2xx without an `errorCode`, otherwise null.
 */
async function capture(file, label, url, options, save = true) {
  const res = await request(url, options);
  const errorCode =
    res.body && typeof res.body === "object" && !Array.isArray(res.body)
      ? res.body.errorCode
      : undefined;
  const ok = res.status >= 200 && res.status < 300 && !errorCode;
  const entry = { file: save ? file : null, endpoint: label, status: res.status, ok };
  if (!ok) entry.error = res.error ?? errorCode ?? `HTTP ${res.status}`;
  manifest.push(entry);
  if (save && res.body !== null) files.set(file, res.body);
  return ok ? res.body : null;
}

function skip(file, label, reason) {
  manifest.push({ file, endpoint: label, skipped: reason });
}

// --- local API (local_api.rs) ----------------------------------------------

const lockfile = readLockfile();
const localBase = `https://127.0.0.1:${lockfile.port}`;
const localOptions = {
  agent: localAgent,
  timeoutMs: LOCAL_TIMEOUT_MS,
  headers: {
    Authorization: `Basic ${Buffer.from(`riot:${lockfile.password}`).toString("base64")}`,
  },
};

function local(file, path, save = true) {
  return capture(file, `local ${path}`, `${localBase}${path}`, localOptions, save);
}

const token = await local(null, "/entitlements/v1/token", false);
const entitlements = token?.accessToken && token?.token && token?.subject ? token : null;
const puuid = entitlements?.subject ?? null;

const regionLocale = await local("region-locale.json", "/riotclient/region-locale");
const presences = await local("presences.json", "/chat/v4/presences");

// --- presence decoding (presence.rs) ---------------------------------------

function isValorant(p) {
  if (p.championId !== undefined) return false;
  if (p.product === "league_of_legends") return false;
  if (p.product === "valorant") return true;
  return typeof p.private === "string" && p.private !== "";
}

// Fields are read nested first, then flat: Riot switches between both shapes.
function dual(decoded, parent, key) {
  return decoded?.[parent]?.[key] ?? decoded?.[key];
}

let ownPresence = null;
const ownRaw = presences?.presences?.find((p) => p.puuid === puuid && isValorant(p));
if (!puuid) {
  skip("own-presence-decoded.json", "decode own private presence", "entitlements unavailable, own puuid unknown");
} else if (ownRaw?.private) {
  try {
    ownPresence = JSON.parse(Buffer.from(ownRaw.private, "base64").toString("utf8"));
    files.set("own-presence-decoded.json", ownPresence);
    manifest.push({ file: "own-presence-decoded.json", endpoint: "decode own private presence", ok: true });
  } catch (err) {
    manifest.push({ file: "own-presence-decoded.json", endpoint: "decode own private presence", ok: false, error: err.message });
  }
} else {
  skip("own-presence-decoded.json", "decode own private presence", "own Valorant presence missing or private blob empty");
}

const presenceState = dual(ownPresence, "matchPresenceData", "sessionLoopState");
const presenceVersion = dual(ownPresence, "partyPresenceData", "partyClientVersion");

// --- remote API (remote_api.rs) --------------------------------------------

// Region -> shard as in constants.rs: latam/br/pbe use the na shard; pbe's glz region is na.
const rawRegion = regionLocale?.region;
let state = { menus: "menus", pregame: "pregame", ingame: "ingame" }[String(presenceState).toLowerCase()];
let matchSource = null;
let currentMatchId = null;
let pods = null;

if (!entitlements || !rawRegion) {
  const reason = entitlements
    ? "region-locale unavailable, remote hosts unknown"
    : "entitlements unavailable, no puuid or tokens";
  for (const name of ["pregame", "coregame", "party", "pods", "mmr", "competitive-updates", "match-details"]) {
    skip(`${name}.json`, name, reason);
  }
} else {
  const region = rawRegion === "pbe" ? "na" : rawRegion;
  const shard = ["latam", "br", "pbe"].includes(rawRegion) ? "na" : rawRegion;
  const pd = `https://pd.${shard}.a.pvp.net`;
  const glz = `https://glz-${region}-1.${shard}.a.pvp.net`;

  // Own presence is authoritative for the client version; valorant-api.com is the fallback.
  let clientVersion = typeof presenceVersion === "string" && presenceVersion ? presenceVersion : null;
  if (!clientVersion) {
    const version = await capture(null, "valorant-api /version", `${VALORANT_API}/version`, { timeoutMs: REMOTE_TIMEOUT_MS }, false);
    clientVersion = version?.data?.riotClientVersion ?? "";
  }

  const remoteOptions = {
    timeoutMs: REMOTE_TIMEOUT_MS,
    headers: {
      Authorization: `Bearer ${entitlements.accessToken}`,
      "X-Riot-Entitlements-JWT": entitlements.token,
      "X-Riot-ClientPlatform": CLIENT_PLATFORM,
      "X-Riot-ClientVersion": clientVersion,
      "User-Agent": USER_AGENT,
    },
  };
  const remote = (file, host, path) => capture(file, `${host === pd ? "pd" : "glz"} ${path}`, `${host}${path}`, remoteOptions);

  // With no readable presence state, both lookups run and the one that answers decides the state.
  const tryPregame = state === "pregame" || state === undefined;
  const tryCoregame = state === "ingame" || state === undefined;

  if (tryPregame) {
    const lookup = await remote("pregame-player.json", glz, `/pregame/v1/players/${puuid}`);
    if (lookup?.MatchID) {
      currentMatchId = lookup.MatchID;
      state = "pregame";
      matchSource = await remote("pregame-match.json", glz, `/pregame/v1/matches/${lookup.MatchID}`);
    } else {
      skip("pregame-match.json", "glz /pregame/v1/matches/{id}", "no pregame match id");
    }
  } else {
    skip("pregame-*.json", "glz /pregame/v1/...", `not in agent select (${state})`);
  }

  if (tryCoregame && currentMatchId) {
    skip("coregame-*.json", "glz /core-game/v1/...", "already in agent select");
  } else if (tryCoregame) {
    const lookup = await remote("coregame-player.json", glz, `/core-game/v1/players/${puuid}`);
    if (lookup?.MatchID) {
      currentMatchId = lookup.MatchID;
      state = "ingame";
      matchSource = await remote("coregame-match.json", glz, `/core-game/v1/matches/${lookup.MatchID}`);
      await remote("coregame-loadouts.json", glz, `/core-game/v1/matches/${lookup.MatchID}/loadouts`);
    } else {
      skip("coregame-*.json", "glz /core-game/v1/matches/{id}[/loadouts]", "no coregame match id");
    }
  } else {
    skip("coregame-*.json", "glz /core-game/v1/...", `not in game (${state})`);
  }

  const partyPlayer = await remote("party-player.json", glz, `/parties/v1/players/${puuid}`);
  if (partyPlayer?.CurrentPartyID) {
    await remote("party.json", glz, `/parties/v1/parties/${partyPlayer.CurrentPartyID}`);
  } else {
    skip("party.json", "glz /parties/v1/parties/{id}", "no current party id");
  }
  // Every game pod the region offers, keyed by pod id: the source of the server-label tokens.
  pods = await remote("pods.json", glz, "/parties/v1/parties/customgameconfigs");

  await remote("mmr.json", pd, `/mmr/v1/players/${puuid}`);
  const updates = await remote(
    "competitive-updates.json",
    pd,
    `/mmr/v1/players/${puuid}/competitiveupdates?startIndex=0&endIndex=${COMPETITIVE_UPDATES_END_INDEX}&queue=competitive`,
  );

  if (withMatchDetails) {
    // match-details only exists once a match has ended, so outside a match the last
    // played match of any queue is used.
    let detailsId = currentMatchId;
    if (!detailsId) {
      const history = await remote("match-history.json", pd, `/match-history/v1/history/${puuid}?startIndex=0&endIndex=1`);
      detailsId = history?.History?.[0]?.MatchID ?? updates?.Matches?.[0]?.MatchID ?? null;
    }
    if (detailsId) {
      await remote("match-details.json", pd, `/match-details/v1/matches/${detailsId}`);
    } else {
      skip("match-details.json", "pd /match-details/v1/matches/{id}", "no current or recent match id");
    }
  } else {
    skip("match-details.json", "pd /match-details/v1/matches/{id}", "pass --match-details to fetch");
  }
}

// --- write -----------------------------------------------------------------

state ??= "unknown";
const stamp = new Date().toISOString().replace(/\..+$/, "").replace(/:/g, "-");
const outDir = join(repoRoot, "captures", `${stamp}_${state}`);
mkdirSync(outDir, { recursive: true });

for (const [file, value] of files) {
  writeFileSync(join(outDir, file), `${JSON.stringify(value, null, 2)}\n`);
}
writeFileSync(join(outDir, "manifest.json"), `${JSON.stringify({ state, presenceState: presenceState ?? null, requests: manifest }, null, 2)}\n`);

// Endpoint path for the console with the query dropped and id-like segments (long, with a
// digit) cut to 8 characters, as remote_api.rs `url_path` does, so the output is shareable.
function shortEndpoint(endpoint) {
  return endpoint
    .split("?")[0]
    .split("/")
    .map((seg) => (seg.length > 8 && /\d/.test(seg) ? seg.slice(0, 8) : seg))
    .join("/");
}

console.log(`snapshot: ${relative(repoRoot, outDir)}`);
for (const entry of manifest) {
  const outcome = entry.skipped
    ? `skipped (${entry.skipped})`
    : entry.ok
      ? `${entry.status ?? "ok"}`
      : `${entry.status ?? "-"} ${entry.error}`;
  const name = entry.file ?? "(not saved)";
  console.log(`  ${name.padEnd(28)} ${shortEndpoint(entry.endpoint)}  ${outcome}`);
}

// Mode identifiers, from presence first, then from the match payload (pregame and
// coregame use different field names).
const m = matchSource ?? {};
const mode = {
  sessionLoopState: presenceState,
  queueId: dual(ownPresence, "matchPresenceData", "queueId") ?? m.QueueID ?? m.MatchmakingData?.QueueID,
  provisioningFlow: dual(ownPresence, "matchPresenceData", "provisioningFlow") ?? m.ProvisioningFlow ?? m.ProvisioningFlowID,
  partyState: dual(ownPresence, "partyPresenceData", "partyState"),
  modeId: m.ModeID ?? m.Mode,
  mapId: m.MapID,
  gamePodId: m.GamePodID,
};
console.log("detected:");
for (const [key, value] of Object.entries(mode)) {
  console.log(`  ${key.padEnd(17)} ${value === "" ? "(empty)" : (value ?? "-")}`);
}
const podIds = Object.keys(pods?.GamePodPingServiceInfo ?? {});
console.log(`  ${"gamePods".padEnd(17)} ${podIds.length || "-"}`);
for (const id of podIds) console.log(`    ${id}`);

if (!entitlements) {
  fail("the local client did not return entitlements yet (still starting?); try again shortly");
}
