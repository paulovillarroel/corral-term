// Corral's install counter. When an installation finishes, `corral install` sends one
// anonymous ping here: the version, the system (linux or wsl), whether it is a new
// install or an update (and on an update, the version it replaced), and where it came
// from (the published package or a clone). When it stops because something required
// is missing, the ping says `stopped` and which checks failed, by the names Corral
// gives them (herdr, r, …); install.sh sends one too when it stops before Corral is
// on the machine (system macos or other, a processor that isn't x86_64, no tar). The
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
// GET /api/installed?token=STATS_TOKEN returns the totals; `updates` counts each
// "<from> -> <version>" ("unknown" when the ping didn't say), and `missing` how many
// stops each check caused.
import { put, list } from "@vercel/blob";

const VERSION = /^\d{1,3}\.\d{1,3}\.\d{1,3}$/;
// The checks a stop can name: corral install's required ones, then install.sh's.
const CHECKS = ["herdr", "herdr_old", "r", "r_packages", "folder", "other", "system", "arch", "basics"];
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
    const kind = pick(body.kind, ["new", "update", "stopped"], "new");
    const source = pick(body.source, ["release", "clone"], "release");
    let extra = "";
    if (kind === "update" && VERSION.test(body.from || "")) extra = `-${body.from}`;
    if (kind === "stopped") {
      const missing = [...new Set((Array.isArray(body.missing) ? body.missing : []).filter((m) => CHECKS.includes(m)))];
      extra = `-${missing.length ? missing.join("+") : "other"}`;
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
    const counts = { total: 0, country: {}, system: {}, kind: {}, source: {}, version: {}, updates: {}, missing: {}, day: {} };
    const add = (group, key) => { counts[group][key] = (counts[group][key] || 0) + 1; };
    let cursor;
    do {
      const page = await list({ prefix: "installs/", cursor, limit: 1000 });
      for (const blob of page.blobs) {
        const [, day, name] = blob.pathname.split("/");
        const [country, system, kind, source, version, sixth] = name.split("-");
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
