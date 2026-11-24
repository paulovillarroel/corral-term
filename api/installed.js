// Corral's install counter. After a successful install, `corral install` sends one
// anonymous ping here: the version, the system (linux or wsl), whether it is a new
// install or an update (and on an update, the version it replaced), and where it came
// from (the published package or a clone). The country comes from Vercel's
// geolocation of the request; the IP itself is never stored. CORRAL_NO_TELEMETRY=1
// (or DO_NOT_TRACK=1) stops the ping.
//
// Each ping is one tiny file in a private Vercel Blob store, and its name carries the
// data, so counting only lists names:
//   installs/<day>/<country>-<system>-<kind>-<source>-<version>[-<from>]-<random>.json
// <from> is there only on updates that said which version they replaced (Corral 0.1.17
// and later). Older pings have none; the random suffix never reads as a version, since
// it has no dots, so both kinds of names count right.
//
// GET /api/installed?token=STATS_TOKEN returns the totals; `updates` counts each
// "<from> -> <version>" ("unknown" when the ping didn't say).
import { put, list } from "@vercel/blob";

const VERSION = /^\d{1,3}\.\d{1,3}\.\d{1,3}$/;
const pick = (value, allowed, fallback) => (allowed.includes(value) ? value : fallback);

export default async function handler(req, res) {
  if (req.method === "POST") {
    let body = req.body;
    if (typeof body === "string") {
      try { body = JSON.parse(body); } catch (e) { body = {}; }
    }
    body = body || {};
    const version = VERSION.test(body.version || "") ? body.version : "unknown";
    const system = pick(body.system, ["linux", "wsl"], "other");
    const kind = pick(body.kind, ["new", "update"], "new");
    const source = pick(body.source, ["release", "clone"], "release");
    const from = kind === "update" && VERSION.test(body.from || "") ? `-${body.from}` : "";
    const country = String(req.headers["x-vercel-ip-country"] || "XX").slice(0, 2).toUpperCase().replace(/[^A-Z]/g, "X");
    const day = new Date().toISOString().slice(0, 10);
    await put(`installs/${day}/${country}-${system}-${kind}-${source}-${version}${from}.json`, "{}", {
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
    const counts = { total: 0, country: {}, system: {}, kind: {}, source: {}, version: {}, updates: {}, day: {} };
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
