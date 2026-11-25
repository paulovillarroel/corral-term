// Corral's install counter. When an installation finishes, `corral install` sends one
// anonymous ping here: the version, the system (linux or wsl), whether it is a new
// install or an update (and on an update, the version it replaced), and where it came
// from (the published package or a clone). When it stops because something required
// is missing, the ping says `stopped` and which checks failed, by the names Corral
// gives them (herdr, r, …); install.sh sends one too when it stops before Corral is
// on the machine (system macos or other, a processor that isn't x86_64, no tar) or
// when a published package doesn't check out (sha256, sha256_missing, attestation).
// `corral uninstall` sends one with kind "uninstall". The
// country comes from Vercel's geolocation of the request; the IP itself is never
// stored. CORRAL_NO_TELEMETRY=1, DO_NOT_TRACK=1 or `corral config telemetry off`
// stop the ping.
//
// Each ping is one tiny file in a private Vercel Blob store, and its name carries the
// data, so counting only lists names:
//   installs/<day>/<country>-<system>-<kind>-<source>-<version>[-<extra>]-<random>.json
// <extra> is the version an update replaced (Corral 0.1.17 and later), or a stop's
// failed checks joined by "+". Older pings have none; the random suffix never reads
// as a version (it has no dots) and stops always carry their list, so every name
// counts right.
//
// Since Corral 0.1.20 an install, update or stop also says where Corral runs, in
// categories that name no person, machine or path: the agent (claude, codex, gemini,
// opencode, none, other), the Ubuntu or Debian release (noble, bookworm…), herdr's
// version, R's major version and whether uv is there. They go in one more segment,
// e_<agent>_<release>_<herdr>_<r>_<python>, which older names never have; each value
// is checked here against a list or a pattern, so nothing else can get in. An
// uninstall says only what every ping says (kind "uninstall").
//
// GET /api/installed?token=STATS_TOKEN returns the totals; `updates` counts each
// "<from> -> <version>" ("unknown" when the ping didn't say), `missing` how many
// stops each check caused, and agent, distro, herdr, r and python the pings that
// said where Corral runs.
import { put, list } from "@vercel/blob";

const VERSION = /^\d{1,3}\.\d{1,3}\.\d{1,3}$/;
// The checks a stop can name: corral install's required ones, then install.sh's.
const CHECKS = ["herdr", "herdr_old", "r", "r_packages", "folder", "other", "system", "arch", "basics",
  "sha256", "sha256_missing", "attestation"];
const AGENTS = ["claude", "codex", "gemini", "opencode", "none", "other"];
const ENV = ["agent", "distro", "herdr", "r", "python"];
const shaped = (value, pattern, fallback) => (typeof value === "string" && pattern.test(value) ? value : fallback);
const pick = (value, allowed, fallback) => (allowed.includes(value) ? value : fallback);

export default async function handler(req, res) {
  if (req.method === "POST") {
    let body = req.body;
    if (typeof body === "string") {
      try { body = JSON.parse(body); } catch (e) { body = {}; }
    }
    body = body || {};
    const version = VERSION.test(body.version || "") ? body.version : "unknown";
    const system = pick(body.system, ["linux", "wsl", "macos"], "other");
    const kind = pick(body.kind, ["new", "update", "stopped", "uninstall"], "new");
    const source = pick(body.source, ["release", "clone"], "release");
    let extra = "";
    if (kind === "update" && VERSION.test(body.from || "")) extra = `-${body.from}`;
    if (kind === "stopped") {
      const missing = [...new Set((Array.isArray(body.missing) ? body.missing : []).filter((m) => CHECKS.includes(m)))];
      extra = `-${missing.length ? missing.join("+") : "other"}`;
    }
    if (kind !== "uninstall" && body.agent !== undefined) {
      const e = [
        pick(body.agent, AGENTS, "other"),
        shaped(body.distro, /^[a-z]{1,16}$/, "other"),
        body.herdr === "none" ? "none" : shaped(body.herdr, /^\d{1,4}(\.\d{1,4}){1,2}$/, "unknown"),
        body.r === "none" ? "none" : shaped(body.r, /^\d{1,2}\.\d{1,2}$/, "unknown"),
        pick(body.python, ["uv", "none"], "none"),
      ];
      extra += `-e_${e.join("_")}`;
    }
    const country = String(req.headers["x-vercel-ip-country"] || "XX").slice(0, 2).toUpperCase().replace(/[^A-Z]/g, "X");
    const day = new Date().toISOString().slice(0, 10);
    await put(`installs/${day}/${country}-${system}-${kind}-${source}-${version}${extra}.json`, "{}", {
      access: "private",
      addRandomSuffix: true,
      contentType: "application/json",
    });
    res.status(204).end();
    return;
  }

  if (req.method === "GET") {
    const token = process.env.STATS_TOKEN;
    if (!token || req.query.token !== token) {
      res.status(404).end();
      return;
    }
    const counts = { total: 0, country: {}, system: {}, kind: {}, source: {}, version: {}, updates: {}, missing: {},
      agent: {}, distro: {}, herdr: {}, r: {}, python: {}, day: {} };
    const add = (group, key) => { counts[group][key] = (counts[group][key] || 0) + 1; };
    let cursor;
    do {
      const page = await list({ prefix: "installs/", cursor, limit: 1000 });
      for (const blob of page.blobs) {
        const [, day, name] = blob.pathname.split("/");
        const [country, system, kind, source, version, ...rest] = name.split("-");
        const sixth = rest[0];
        const env = rest.find((r) => r.startsWith("e_"));
        if (env) env.slice(2).split("_").forEach((v, i) => ENV[i] && add(ENV[i], v));
        counts.total += 1;
        add("day", day);
        add("country", country);
        add("system", system);
        add("kind", kind);
        add("source", source);
        add("version", version);
        if (kind === "update") add("updates", `${VERSION.test(sixth || "") ? sixth : "unknown"} -> ${version}`);
        if (kind === "stopped") for (const m of (sixth || "other").split("+")) add("missing", m);
      }
      cursor = page.hasMore ? page.cursor : undefined;
    } while (cursor);
    res.setHeader("Cache-Control", "no-store");
    res.status(200).json(counts);
    return;
  }

  res.setHeader("Allow", "GET, POST");
  res.status(405).end();
}
