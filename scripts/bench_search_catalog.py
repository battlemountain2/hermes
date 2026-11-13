"""Generated SQLite search experiment; never reads files from a cataloged path."""

import argparse
import bisect
import json
import math
import platform
import signal
import sqlite3
import statistics
import tempfile
import time
from pathlib import Path


PAGE_LIMIT = 100
CANDIDATE_LIMIT = 500
BATCH_SIZE = 1000
MAX_DB_BYTES = 192 * 1024 * 1024
WORDS = ("aurora", "harbor", "forest", "cobalt", "meadow", "orbit", "canyon", "velvet")
KINDS = (("document", "pdf"), ("audio", "flac"), ("image", "jpg"), ("video", "mp4"))
SELECT_COLUMNS = "id, name, path, title, artist, album, kind, ext, modified, size"


def records(count):
    result = []
    for number in range(1, count + 1):
        word = WORDS[number % len(WORDS)]
        kind, extension = KINDS[number % len(KINDS)]
        stem = f"{word}-report-{number:06d}"
        if number % 101 == 0:
            stem = f"microarchitecture-{number:06d}"
        if number == 17:
            stem = 'Café 東京 "Live" 100%_mix'
        name = f"{stem}.{extension}"
        result.append((number, name, f"/synthetic/root{number % 3}/folder{number % 97}/{name}",
                       f"{word} session {number:06d}", f"Artist {number % 257:03d}",
                       f"Album {number % 1009:04d}", kind, extension,
                       1_700_000_000 + number * 60, 1024 + number * 101))
    return result


def quote_fts(value):
    return '"' + value.replace('"', '""') + '"'


def measure(operation, repeats):
    operation()
    samples = []
    for _ in range(repeats):
        started = time.perf_counter()
        result = operation()
        samples.append((time.perf_counter() - started) * 1000)
    return {"p50_ms": round(statistics.median(samples), 3),
            "p95_ms": round(sorted(samples)[math.ceil(0.95 * repeats) - 1], 3),
            "samples": repeats, "returned": len(result)}


def connect(path, readonly=False):
    connection = sqlite3.connect(f"{path.as_uri()}?mode=ro" if readonly else str(path), uri=readonly)
    connection.execute("PRAGMA cache_size=-8192")
    connection.execute("PRAGMA temp_store=MEMORY")
    connection.execute("PRAGMA mmap_size=0")
    if not readonly:
        connection.execute("PRAGMA page_size=4096")
        connection.execute(f"PRAGMA max_page_count={MAX_DB_BYTES // 4096}")
        connection.execute("PRAGMA journal_mode=DELETE")
        connection.execute("PRAGMA synchronous=FULL")
    return connection


def build(path, rows):
    connection = connect(path)
    stages = {}
    try:
        started = time.perf_counter()
        connection.executescript("""
            CREATE TABLE catalog (
                id INTEGER PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE,
                title TEXT, artist TEXT, album TEXT, kind TEXT, ext TEXT,
                modified INTEGER, size INTEGER);
            CREATE INDEX exact_artist ON catalog(artist, id);
            CREATE INDEX exact_album ON catalog(album, id);
            CREATE INDEX exact_title ON catalog(title, id);
            CREATE INDEX exact_name ON catalog(name, id);
            CREATE INDEX typed_date ON catalog(kind, ext, modified, id);
        """)
        for offset in range(0, len(rows), BATCH_SIZE):
            with connection:
                connection.executemany("INSERT INTO catalog VALUES (?,?,?,?,?,?,?,?,?,?)",
                                       rows[offset:offset + BATCH_SIZE])
        stages["base"] = {"seconds": round(time.perf_counter() - started, 3),
                          "bytes": path.stat().st_size}
        for table, declaration, columns, indexes in (
            ("tokens", "tokenize='unicode61 remove_diacritics 2', prefix='2 3 4'",
             "name, path, title, artist, album", (0, 1, 2, 3, 4, 5)),
            ("grams", "tokenize='trigram'", "name", (0, 1)),
        ):
            started = time.perf_counter()
            connection.execute(f"CREATE VIRTUAL TABLE {table} USING fts5({columns}, "
                               f"content='catalog', content_rowid='id', {declaration})")
            placeholders = ",".join("?" for _ in indexes)
            for offset in range(0, len(rows), BATCH_SIZE):
                with connection:
                    connection.executemany(
                        f"INSERT INTO {table}(rowid,{columns}) VALUES ({placeholders})",
                        (tuple(row[index] for index in indexes)
                         for row in rows[offset:offset + BATCH_SIZE]))
            with connection:
                connection.execute(f"INSERT INTO {table}({table}) VALUES ('optimize')")
            stages[table] = {"seconds": round(time.perf_counter() - started, 3),
                             "bytes": path.stat().st_size}
        for table in ("tokens", "grams"):
            with connection:
                connection.execute(f"INSERT INTO {table}({table},rank) VALUES ('integrity-check',1)")
        if connection.execute("PRAGMA integrity_check").fetchone() != ("ok",):
            raise RuntimeError("Database integrity check failed")
    finally:
        connection.close()
    return stages


def prepared(rows):
    return [(row[0], row[1].lower(), row[2].lower(),
             " ".join(row[3:6]).lower()) for row in rows]


def score_row(row, query):
    identity, name, path, metadata = row
    position = name.find(query)
    if position >= 0:
        score = 10_000 - position * 12 - len(name)
    elif (position := path.find(query)) >= 0:
        score = 7_000 - position * 4 - len(path)
    elif all(term in metadata or term in name or term in path for term in query.split()):
        score = 8_000 - len(metadata)
    else:
        score = 1000
        previous = -1
        for wanted in query:
            position = path.find(wanted, previous + 1)
            if position < 0:
                return None
            score -= position
            if previous >= 0 and previous + 1 == position:
                score += 80
            if position == 0 or path[position - 1] in "/-_ .":
                score += 45
            previous = position
    return score + (20_000 if name == query else 0)


def linear_top(rows, query):
    matches = []
    for row in rows:
        score = score_row(row, query)
        if score is not None:
            entry = (-score, row[2], row[0])
            position = bisect.bisect_left(matches, entry)
            if position < PAGE_LIMIT:
                matches.insert(position, entry)
                del matches[PAGE_LIMIT:]
    return matches


def substring_candidates(connection, needle):
    if len(needle) < 3:
        return [], False
    rows = connection.execute(
        f"SELECT {SELECT_COLUMNS} FROM catalog WHERE id IN "
        "(SELECT rowid FROM grams WHERE grams MATCH ? ORDER BY rowid LIMIT ?)",
        (quote_fts(needle), CANDIDATE_LIMIT + 1)).fetchall()
    rows.sort(key=lambda row: row[0])
    truncated = len(rows) > CANDIDATE_LIMIT
    verified = [row for row in rows[:CANDIDATE_LIMIT] if needle.lower() in row[1].lower()]
    return linear_top(prepared(verified), needle.lower()), truncated


def benchmark(count, repeats):
    rows = records(count)
    with tempfile.TemporaryDirectory(prefix="hermes-search-bench-") as directory:
        path = Path(directory) / "catalog.sqlite"
        stages = build(path, rows)
        started = time.perf_counter()
        connection = connect(path, readonly=True)
        try:
            if connection.execute("SELECT count(*) FROM catalog").fetchone()[0] != count:
                raise RuntimeError("Reopen lost catalog rows")
            first_ids = connection.execute(
                "SELECT rowid FROM tokens WHERE tokens MATCH ? ORDER BY rowid LIMIT 100",
                (quote_fts("aurora"),)).fetchall()
            reopen_ms = (time.perf_counter() - started) * 1000
            expected_token = [row[0] for row in rows if row[0] % 8 == 0]
            assert [row[0] for row in first_ids] == expected_token[:100]
            token_sql = "SELECT rowid FROM tokens WHERE tokens MATCH ? ORDER BY rowid LIMIT 100"
            cases = {
                "token_aurora": (token_sql, (quote_fts("aurora"),)),
                "prefix_aur": (token_sql, (quote_fts("aur") + "*",)),
                "token_no_hit": (token_sql, (quote_fts("zzzzmissing"),)),
                "token_broad_report": (token_sql, (quote_fts("report"),)),
                "bm25_broad_report": ("SELECT rowid FROM tokens WHERE tokens MATCH ? "
                                      "ORDER BY bm25(tokens), rowid LIMIT 100", (quote_fts("report"),)),
                "exact_artist": ("SELECT id FROM catalog WHERE artist=? ORDER BY id LIMIT 100",
                                 ("Artist 042",)),
                "exact_album": ("SELECT id FROM catalog WHERE album=? ORDER BY id LIMIT 100",
                                ("Album 0042",)),
                "exact_title": ("SELECT id FROM catalog WHERE title=? ORDER BY id LIMIT 100",
                                ("forest session 000042",)),
                "typed_date_size": ("SELECT id FROM catalog WHERE kind=? AND ext=? "
                                    "AND modified>=? AND modified<? AND size>=? "
                                    "ORDER BY modified,id LIMIT 100",
                                    ("document", "pdf", 1_700_060_000, 1_700_600_000, 200_000)),
                "like_infix_scan": ("SELECT id FROM catalog WHERE name LIKE ? ORDER BY id LIMIT 100",
                                    ("%croarch%",)),
                "like_short_scan": ("SELECT id FROM catalog WHERE name LIKE ? ORDER BY id LIMIT 100",
                                    ("%or%",)),
                "like_no_hit_scan": ("SELECT id FROM catalog WHERE name LIKE ? ORDER BY id LIMIT 100",
                                     ("%zzzzmissing%",)),
            }
            expected = {
                "token_aurora": expected_token, "prefix_aur": expected_token,
                "token_no_hit": [],
                "token_broad_report": [row[0] for row in rows if "report" in row[1]],
                "exact_artist": [row[0] for row in rows if row[4] == "Artist 042"],
                "exact_album": [row[0] for row in rows if row[5] == "Album 0042"],
                "exact_title": [row[0] for row in rows if row[3] == "forest session 000042"],
                "typed_date_size": [row[0] for row in rows if row[6:8] == ("document", "pdf")
                                    and 1_700_060_000 <= row[8] < 1_700_600_000 and row[9] >= 200_000],
                "like_infix_scan": [row[0] for row in rows if "croarch" in row[1]],
                "like_short_scan": [row[0] for row in rows if "or" in row[1]],
                "like_no_hit_scan": [],
            }
            for label, identities in expected.items():
                assert [row[0] for row in connection.execute(*cases[label])] == identities[:100], label
            timings = {label: measure(lambda sql=sql, args=args: connection.execute(sql, args).fetchall(),
                                      repeats) for label, (sql, args) in cases.items()}
            substring_evidence = {}
            for needle in ("croarch", "report", "zzzzmissing", '"Live"', "100%_", "東京", "or"):
                matches, truncated = substring_candidates(connection, needle)
                oracle = [row for row in rows if needle.lower() in row[1].lower()]
                if len(needle) >= 3:
                    assert matches == linear_top(prepared(oracle[:CANDIDATE_LIMIT]), needle.lower())
                    assert truncated == (len(oracle) > CANDIDATE_LIMIT)
                substring_evidence[needle] = {"total_matches": len(oracle), "returned": len(matches),
                                              "truncated": truncated, "short_query_deferred": len(needle) < 3}
                if needle in ("croarch", "report", "zzzzmissing"):
                    timings[f"trigram_{needle}_rerank"] = measure(
                        lambda needle=needle: substring_candidates(connection, needle)[0], repeats)
            for text, expected_ids in (("cafe", [17]), ("東京", [17]), ('"Live"', [17]),
                                       ("aurra", []), ("croarch", [])):
                assert [row[0] for row in connection.execute(token_sql, (quote_fts(text),))] == expected_ids
            assert score_row(prepared([rows[7]])[0], "aurra") is not None
            proxy = prepared(rows)
            for limit in sorted({min(count, 40_000), count}):
                subset = proxy[:limit]
                for query in ("aurora", "croarch", "aurra", "zzzzmissing"):
                    timings[f"python_linear_{limit}_{query}"] = measure(
                        lambda query=query: linear_top(subset, query), repeats)
            plans = {label: [row[3] for row in connection.execute("EXPLAIN QUERY PLAN " + sql, args)]
                     for label, (sql, args) in cases.items()}
            return {"records": count, "stages": stages, "reopen_count_first_token_ms": round(reopen_ms, 3),
                    "checks": "PASS: persistence, integrity, exact sets, substring bounds, Unicode, literals, non-fuzzy FTS",
                    "substring_evidence": substring_evidence, "timings": timings, "plans": plans}
        finally:
            connection.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sizes", nargs="+", type=int, choices=(10_000, 100_000), default=[10_000, 100_000])
    parser.add_argument("--repeats", type=int, choices=range(5, 52), default=21)
    args = parser.parse_args()

    def timeout(signum, frame):
        raise TimeoutError("Benchmark exceeded its 180-second budget")

    signal.signal(signal.SIGALRM, timeout)
    signal.alarm(180)
    started = time.perf_counter()
    print(json.dumps({"python": platform.python_version(), "sqlite": sqlite3.sqlite_version,
                      "platform": platform.platform(), "temp_parent": tempfile.gettempdir(),
                      "cache_kib": 8192, "max_db_bytes": MAX_DB_BYTES,
                      "candidate_limit": CANDIDATE_LIMIT, "batch_size": BATCH_SIZE}), flush=True)
    try:
        for count in sorted(set(args.sizes)):
            print(json.dumps(benchmark(count, args.repeats), indent=2), flush=True)
    finally:
        signal.alarm(0)
    print(json.dumps({"elapsed_seconds": round(time.perf_counter() - started, 3),
                      "temporary_catalogs_removed": True}), flush=True)


if __name__ == "__main__":
    main()
