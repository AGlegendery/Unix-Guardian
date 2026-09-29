#!/usr/bin/env python3
"""Checks every Steam App ID in data/games.json against the Steam store API.

    ./scripts/verify-games-db.py            # report mismatches, exit 1 if any

A mismatch means the ID points at a different game (or at nothing) and would
attach the wrong profile to whatever the user launches.
"""
import json, pathlib, sys, time, urllib.request

db = json.loads((pathlib.Path(__file__).parent.parent / "data/games.json").read_text())
bad = 0
for g in db["games"]:
    appid = g.get("steam_appid")
    if not appid:
        continue
    url = f"https://store.steampowered.com/api/appdetails?appids={appid}&filters=basic&l=english"
    req = urllib.request.Request(url, headers={"User-Agent": "unix-guardian-dbcheck"})
    with urllib.request.urlopen(req, timeout=15) as r:
        d = json.load(r).get(str(appid), {})
    name = d.get("data", {}).get("name") if d.get("success") else None
    if name is None or d["data"].get("type") != "game":
        print(f"MISSING  {appid:>8}  {g['id']}: Steam has no game with this ID")
        bad += 1
    elif name != g["name"]:
        print(f"RENAMED  {appid:>8}  {g['id']}: ours {g['name']!r}, Steam {name!r}")
    time.sleep(0.4)
print(f"checked; {bad} broken")
sys.exit(1 if bad else 0)
