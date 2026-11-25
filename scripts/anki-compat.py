#!/usr/bin/env python3
"""Compare Anki and ranki on real .apkg files.

All collections live under .tmp/anki-compat. The user's Anki collection and
Wayland session are never touched.
"""
import argparse
import hashlib
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
import time
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
WORK = ROOT / ".tmp" / "anki-compat"
BIN = ROOT / "target" / "debug" / "ranki"
NOW = 1_900_000_000
CATEGORIES = (
    "arabic chinese english french german hebrew japanese korean russian "
    "spanish anatomy biology chemistry geography history law math music "
    "pathology physics"
).split()
BASE = "https://ankiweb.net"


def log(*parts):
    print(*parts, flush=True)


def get(path, attempts=8):
    for attempt in range(attempts):
        try:
            return urllib.request.urlopen(BASE + path, timeout=30).read()
        except urllib.error.HTTPError as error:
            if error.code != 429 or attempt == attempts - 1:
                raise
            delay = min(60, 10 * (attempt + 1))
            log(f"AnkiWeb rate limited, waiting {delay}s")
            time.sleep(delay)
    raise RuntimeError("unreachable")


def varint(data, pos):
    value = shift = 0
    while True:
        byte = data[pos]
        pos += 1
        value |= (byte & 0x7F) << shift
        if byte < 0x80:
            return value, pos
        shift += 7


def proto(data):
    fields = {}
    pos = 0
    while pos < len(data):
        tag, pos = varint(data, pos)
        number, wire = tag >> 3, tag & 7
        if wire == 0:
            value, pos = varint(data, pos)
        elif wire == 2:
            size, pos = varint(data, pos)
            value = data[pos:pos + size]
            pos += size
        elif wire == 1:
            value, pos = data[pos:pos + 8], pos + 8
        elif wire == 5:
            value, pos = data[pos:pos + 4], pos + 4
        else:
            raise ValueError("unsupported protobuf wire type")
        fields.setdefault(number, []).append(value)
    return fields


def catalog():
    WORK.mkdir(parents=True, exist_ok=True)
    decks = []
    for category in CATEGORIES:
        rows = proto(get("/svc/shared/list-decks?search=" +
                         urllib.parse.quote(category))).get(1, [])
        ranked = sorted(rows, key=lambda row: proto(row).get(3, [0])[0],
                        reverse=True)[:10]
        for row in ranked:
            entry = proto(row)
            decks.append({
                "category": category,
                "id": entry[1][0],
                "title": entry[2][0].decode(errors="replace"),
                "score": proto(row).get(3, [0])[0],
            })
        log(f"{category}: {len(rows)} decks, kept {len(ranked)}")
        time.sleep(5)
    (WORK / "catalog.json").write_text(json.dumps(decks, indent=2))
    log(f"catalog: {len(decks)} decks")


def download():
    catalog = WORK / "catalog_web.json"
    if not catalog.exists():
        catalog = WORK / "catalog.json"
    decks = json.loads(catalog.read_text())
    for rank, deck in enumerate(decks):
        deck.setdefault("rank", rank + 1)
    folder = WORK / "decks"
    folder.mkdir(parents=True, exist_ok=True)
    for deck in decks:
        target = folder / f"{deck['category']}_{deck['rank']}_{deck['id']}.apkg"
        if target.exists():
            continue
        info = proto(get(f"/svc/shared/item-info?sharedId={deck['id']}"))
        available = proto(info[1][0])
        key = proto(available[10][0])[5][0].decode()
        target.write_bytes(get(
            f"/svc/shared/download-deck/{deck['id']}?t={key}"))
        log(f"downloaded {target.name} {target.stat().st_size} bytes")
        time.sleep(2)


def ranki(*args, env=None):
    variables = os.environ.copy()
    variables.update(env or {})
    return subprocess.run([str(BIN), "--no-sync", *map(str, args)],
                          env=variables, check=True, capture_output=True,
                          text=True).stdout


def ranki_env(path):
    config = WORK / "config"
    (config / "ranki").mkdir(parents=True, exist_ok=True)
    return {
        "XDG_CONFIG_HOME": str(config),
        "RANKI_DEV_NOW_SECS": str(NOW),
        "collection": str(path),
    }


def import_ranki(package, path):
    if path.exists():
        path.unlink()
    env = ranki_env(path)
    ranki("--collection", path, "import", package, env=env)


def anki_import(col, package):
    from anki.collection import ImportAnkiPackageRequest, ImportAnkiPackageOptions
    col.import_anki_package(ImportAnkiPackageRequest(
        package_path=str(package),
        options=ImportAnkiPackageOptions(
            merge_notetypes=True, with_scheduling=True,
            with_deck_configs=True)))


def clean_json(value):
    if not value:
        return "{}"
    try:
        value = json.loads(value)
    except (TypeError, ValueError):
        return value

    def normalize(item):
        if isinstance(item, float) and item.is_integer():
            return int(item)
        if isinstance(item, float):
            return round(item, 3)
        if isinstance(item, dict):
            return {key: normalize(sub) for key, sub in item.items()}
        if isinstance(item, list):
            return [normalize(sub) for sub in item]
        return item

    return json.dumps(normalize(value), sort_keys=True)


def snapshot(path):
    conn = sqlite3.connect(path)
    conn.row_factory = sqlite3.Row
    models = {
        str(row["id"]): row["name"].rstrip("+")
        for row in conn.execute("SELECT id, name FROM notetypes")
    } if conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' "
                      "AND name='notetypes'").fetchone() else {}
    if not models:
        raw = conn.execute("SELECT models FROM col WHERE id=1").fetchone()[0]
        for key, entry in json.loads(raw or "{}").items():
            models[key] = entry.get("name", "").rstrip("+")
    decks = {}
    if conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' "
                    "AND name='decks'").fetchone():
        for row in conn.execute("SELECT id, name FROM decks"):
            decks[str(row["id"])] = row["name"].replace("\x1f", "::")
    if not decks:
        raw = conn.execute("SELECT decks FROM col WHERE id=1").fetchone()[0]
        for key, entry in json.loads(raw or "{}").items():
            decks[key] = entry.get("name", "").replace("\x1f", "::")
    notes = {}
    for row in conn.execute("SELECT id, guid, mid, tags, flds, sfld, csum, "
                            "flags, data FROM notes"):
        notes[str(row["id"])] = (models.get(str(row["mid"]), str(row["mid"])),
                                 row["guid"], row["tags"], row["flds"],
                                 str(row["sfld"]), row["csum"], row["flags"],
                                 row["data"])
    cards = {}
    for row in conn.execute("SELECT id, nid, did, ord, type, queue, due, ivl, "
                            "factor, reps, lapses, left, odue, odid, flags, "
                            "data FROM cards"):
        key = notes.get(str(row["nid"]), ("", str(row["nid"])))[1] + \
            f":{row['ord']}"
        due = 0 if row["queue"] in (1, 4) else row["due"]
        cards[key] = (decks.get(str(row["did"]), str(row["did"])),
                      row["type"], row["queue"], due, row["ivl"],
                      row["factor"], row["reps"], row["lapses"], row["left"],
                      row["odue"], row["odid"], row["flags"],
                      clean_json(row["data"]))
    logs = {}
    indexes = {}
    for row in conn.execute("SELECT id, cid, ease, ivl, lastIvl, factor, time, "
                            "type FROM revlog ORDER BY cid, id"):
        card = conn.execute("SELECT nid, ord FROM cards WHERE id=?",
                            (row["cid"],)).fetchone()
        if card:
            guid = conn.execute("SELECT guid FROM notes WHERE id=?",
                                (card["nid"],)).fetchone()[0]
            base = f"{guid}:{card['ord']}"
            index = indexes.get(base, 0)
            indexes[base] = index + 1
            logs[f"{base}:{index}"] = (row["ease"], row["ivl"],
                                       row["lastIvl"], row["factor"],
                                       row["time"], row["type"])
    conn.close()
    return {"notes": notes, "cards": cards, "logs": logs,
            "decks": sorted(decks.values())}


def diff(before, after, label):
    changes = []
    for section in ("notes", "cards", "logs"):
        old, new = before[section], after[section]
        for key in sorted(set(old) | set(new)):
            if old.get(key) != new.get(key):
                changes.append(f"{label} {section} {key}: "
                               f"{old.get(key)} != {new.get(key)}")
    if before["decks"] != after["decks"]:
        changes.append(f"{label} decks: {before['decks']} != {after['decks']}")
    return changes


def review_anki(path, ratings):
    from anki.collection import Collection
    from anki.scheduler.v3 import CardAnswer
    col = Collection(str(path))
    results = []
    seen = set()
    queue = iter(_anki_queue(col))
    for rating in ratings:
        for _ in range(100):
            try:
                queued, states = next(queue)
            except StopIteration:
                break
            cid = queued[0].card.id
            rows = col.db.execute(
                "SELECT n.guid, c.ord FROM cards c JOIN notes n ON n.id=c.nid "
                "WHERE c.id=?", cid)
            row = rows[0] if rows else (cid, 0)
            key = f"{row[0]}:{row[1]}"
            if key in seen:
                continue
            seen.add(key)
            col.decks.select(queued[0].card.deck_id)
            current = col.sched.get_queued_cards(fetch_limit=1).cards[0]
            chosen = {1: states.again, 2: states.hard, 3: states.good,
                      4: states.easy}[rating]
            col.sched.answer_card(CardAnswer(
                card_id=current.card.id, current_state=current.states.current,
                new_state=chosen, rating=rating - 1,
                answered_at_millis=NOW * 1000,
                milliseconds_taken=1000))
            results.append({"key": key, "rating": rating})
            break
    col.close()
    return results


def _anki_queue(col):
    for deck in col.decks.all_names_and_ids():
        deck_id = deck.id
        col.decks.select(deck_id)
        while True:
            queue = col.sched.get_queued_cards(fetch_limit=1).cards
            if not queue:
                break
            yield queue, queue[0].states


def review_ranki(path, ratings, keys):
    env = ranki_env(path)
    for rating, key in zip(ratings, keys):
        due = ranki("--collection", path, "due", "--limit", "100000", env=env)
        lookup = {}
        for line in due.splitlines():
            cid, did, _ = line.split("\t")
            lookup[cid] = did
        conn = sqlite3.connect(path)
        found = None
        for cid in lookup:
            row = conn.execute(
                "SELECT n.guid, c.ord FROM cards c JOIN notes n ON n.id=c.nid "
                "WHERE c.id=?", (cid,)).fetchone()
            if row and f"{row[0]}:{row[1]}" == key:
                found = cid
                break
        conn.close()
        if found is None:
            raise RuntimeError(f"ranki has no due card {key}")
        ranki("--collection", path, "answer", found, rating, env=env)
    return keys[:len(ratings)]


def compare_package(package, root, label, reviews=6):
    from anki.collection import Collection
    log(f"=== {label}: {package.name}")
    global NOW
    NOW = int(time.time())
    anki_path = root / f"{label}_anki.anki2"
    ranki_path = root / f"{label}_ranki.anki2"
    if anki_path.exists():
        anki_path.unlink()
    col = Collection(str(anki_path))
    anki_import(col, package)
    col.close()
    import_ranki(package, ranki_path)
    before_anki = snapshot(anki_path)
    before_ranki = snapshot(ranki_path)
    findings = diff(before_anki, before_ranki, "after import")

    ratings = [1 + byte % 4 for byte in os.urandom(reviews)]
    reviewed = []
    try:
        reviewed = review_anki(anki_path, ratings)
    except Exception as error:
        findings.append(f"Anki review failed: {error}")
    keys = [item["key"] for item in reviewed] or \
        list(before_ranki["cards"])[:reviews]
    try:
        review_ranki(ranki_path, [item["rating"] for item in reviewed] or ratings,
                     keys)
    except Exception as error:
        findings.append(f"ranki review failed: {error}")
    findings.extend(diff(snapshot(anki_path), snapshot(ranki_path),
                         "after review"))

    anki_export = root / f"{label}_anki.apkg"
    ranki_export = root / f"{label}_ranki.apkg"
    if anki_export.exists():
        anki_export.unlink()
    from anki.collection import ExportAnkiPackageOptions
    col = Collection(str(anki_path))
    col.export_anki_package(out_path=str(anki_export),
                            options=ExportAnkiPackageOptions(
                                with_scheduling=True, with_deck_configs=True), limit=None)
    col.close()
    env = ranki_env(ranki_path)
    ranki("--collection", ranki_path, "export", ranki_export, env=env)
    a_hash = hashlib.sha256(anki_export.read_bytes()).hexdigest()
    r_hash = hashlib.sha256(ranki_export.read_bytes()).hexdigest()
    if a_hash != r_hash:
        findings.append(f"export bytes differ: anki={a_hash} ranki={r_hash}")

    for name, exported in (("anki", anki_export), ("ranki", ranki_export)):
        imported = {}
        for target in ("anki", "ranki"):
            path = root / f"{label}_{name}_into_{target}.anki2"
            if path.exists():
                path.unlink()
            if target == "anki":
                col = Collection(str(path))
                try:
                    anki_import(col, exported)
                finally:
                    col.close()
            else:
                import_ranki(exported, path)
            imported[target] = snapshot(path)
        findings.extend(diff(imported["anki"], imported["ranki"],
                             f"reimport {name}"))
    log("\n".join(findings[:40]) if findings else "no differences")
    return {"label": label, "findings": findings,
            "ratings": ratings, "cards": len(before_ranki["cards"])}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=["catalog", "download", "compare"])
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--pkg", type=pathlib.Path)
    args = parser.parse_args()
    WORK.mkdir(parents=True, exist_ok=True)
    if args.mode == "catalog":
        catalog()
    elif args.mode == "download":
        download()
    else:
        root = WORK / "compare"
        root.mkdir(parents=True, exist_ok=True)
        packages = [args.pkg] if args.pkg else sorted(
            (WORK / "decks").glob("*.apkg"))
        if args.limit:
            packages = packages[:args.limit]
        report = [compare_package(pkg, root, pkg.stem) for pkg in packages]
        (WORK / "report.json").write_text(json.dumps(report, indent=2))
        log(f"{sum(len(item['findings']) for item in report)} findings")


if __name__ == "__main__":
    main()
