#!/usr/bin/env python3
"""Generates the synthetic "Acme Outfitters" example (a fictional outdoor-gear retailer).

Writes, under reference/examples/acme-outfitters/:

- data/*.csv: synthetic source data with deliberate noise (blanks, unmatched keys,
  duplicates, stray whitespace and casing) for the M cleaning steps;
- AcmeOutfitters.pbip with a TMDL semantic model and three PBIR reports;
- blob-map.json: the fictional blob container mapped to data/*.csv (offline only);
- expected.json and EXPECTED.md: values computed here in plain Python over the generated
  rows, independently of the DashCraft engines.

Python 3 standard library only. Deterministic: a fixed seed and no wall clock, so two runs
give byte-identical files. README.md in the output folder is hand-written and kept.

Usage: python3 scripts/examples/gen_acme.py [--out DIR]
"""

from __future__ import annotations

import argparse
import calendar
import csv
import hashlib
import io
import json
import random
import shutil
import uuid
from datetime import date, timedelta
from decimal import ROUND_HALF_UP, Decimal
from pathlib import Path

SEED = 20251231
ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUT = ROOT / "reference" / "examples" / "acme-outfitters"

CONTAINER_URL = "https://acmeoutfittersdemo.blob.core.windows.net/sample-data"
MODEL_NAME = "AcmeOutfitters"
DATA_AS_OF = date(2025, 12, 31)
CAL_START, CAL_END = date(2024, 1, 1), date(2025, 12, 31)
ID_NAMESPACE = uuid.UUID("6f1c2a40-3d1e-4c55-9a7e-0ac3e0000000")

BLOBS = {
    "orders": ("sales/orders.csv", "data/orders.csv"),
    "customers": ("sales/customers.csv", "data/customers.csv"),
    "returns": ("sales/returns.csv", "data/returns.csv"),
    "products": ("catalog/products.csv", "data/products.csv"),
    "stores": ("stores/stores.csv", "data/stores.csv"),
    "targets": ("planning/targets.csv", "data/targets.csv"),
    "web_sessions": ("web/web_sessions.csv", "data/web_sessions.csv"),
}

REPORT_NAMES = {
    "overview": "Acme Outfitters - Overview",
    "stores": "Acme Outfitters - Stores",
    "online": "Acme Outfitters - Online",
}

GENERATED = [
    ".gitignore",
    "data",
    f"{MODEL_NAME}.SemanticModel",
    f"{MODEL_NAME}.pbip",
    *[f"{name}.Report" for name in REPORT_NAMES.values()],
    "blob-map.json",
    "expected.json",
    "EXPECTED.md",
]


# ---------------------------------------------------------------------------
# Small helpers
# ---------------------------------------------------------------------------


def guid(*parts: str) -> str:
    return str(uuid.uuid5(ID_NAMESPACE, "/".join(parts)))


def hex20(*parts: str) -> str:
    return hashlib.sha1("/".join(parts).encode()).hexdigest()[:20]


def dim(y: int, m: int) -> int:
    return calendar.monthrange(y, m)[1]


def D(text: str) -> Decimal:
    return Decimal(text)


def money(x: Decimal) -> Decimal:
    return x.quantize(Decimal("0.01"), rounding=ROUND_HALF_UP)


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)


def write_json(path: Path, obj) -> None:
    write_text(path, json.dumps(obj, indent=2, ensure_ascii=False) + "\n")


def write_csv(path: Path, header: list[str], rows: list[list[str]]) -> None:
    buf = io.StringIO()
    w = csv.writer(buf, lineterminator="\n")
    w.writerow(header)
    w.writerows(rows)
    write_text(path, buf.getvalue())


def iso(d: date | None) -> str:
    return d.isoformat() if d else ""


CALENDAR = [CAL_START + timedelta(days=i) for i in range((CAL_END - CAL_START).days + 1)]
CAL_SET = frozenset(CALENDAR)


# ---------------------------------------------------------------------------
# Source data
# ---------------------------------------------------------------------------

FIRST = [
    "Ana", "Ben", "Carla", "Dev", "Elena", "Felix", "Grace", "Hugo", "Iris", "Jonah",
    "Kira", "Liam", "Maya", "Noah", "Olive", "Pablo", "Quinn", "Rosa", "Sam", "Tara",
    "Uma", "Victor", "Wren", "Xavier", "Yara", "Zane", "Amir", "Bianca", "Caleb", "Dana",
    "Eli", "Fiona", "Gabe", "Hana", "Ivan", "Jade", "Kofi", "Lena", "Marco", "Nadia",
]
LAST = [
    "Adams", "Baker", "Chen", "Diaz", "Evans", "Flores", "Garcia", "Hughes", "Ito", "Jensen",
    "Khan", "Lopez", "Moore", "Nguyen", "O'Brien", "Patel", "Quintero", "Rivera", "Silva", "Turner",
    "Ueda", "Vargas", "Walsh", "Xu", "Young", "Zimmer", "Abbott", "Brooks", "Castillo", "Duarte",
    "Ellis", "Fischer", "Grant", "Holt", "Ibarra", "Jordan", "Kim", "Larsen", "Mendez", "Novak",
]
ACCOUNT_MANAGERS = [
    "Ana Ruiz", "O'Neil Park", "Priya Shah", "Marcus Bell", "Lena Ford", "Tomas Vega",
    "Jo Kim", "D'Arcy Lane",
]
CUSTOMER_CITIES = [
    ("Seattle", "WA"), ("Tacoma", "WA"), ("Portland", "OR"), ("Eugene", "OR"), ("Boise", "ID"),
    ("Denver", "CO"), ("Boulder", "CO"), ("Minneapolis", "MN"), ("Kansas City", "MO"),
    ("Boston", "MA"), ("Burlington", "VT"), ("Philadelphia", "PA"), ("Asheville", "NC"),
    ("Austin", "TX"), ("Atlanta", "GA"), ("Salt Lake City", "UT"), ("Phoenix", "AZ"),
    ("Madison", "WI"), ("Richmond", "VA"), ("Albuquerque", "NM"),
]
EMAIL_DOMAINS = ["mail.example", "inbox.example", "trailpost.example"]

# id, name, region, channel, city, state, manager, open date, square feet
STORES = [
    ("S01", "Seattle Flagship", "West", "Retail", "Seattle", "WA", "riley.chen", "2012-04-14", 32000),
    ("S02", "Portland Pearl", "West", "Retail", "Portland", "OR", "morgan.price", "2014-09-06", 18000),
    ("S03", "Boise Outlet", "West", "Outlet", "Boise", "ID", "casey.nguyen", "2017-03-18", 12000),
    ("S04", "Denver Union", "Central", "Retail", "Denver", "CO", "jamie.ortiz", "2013-05-11", 24000),
    ("S05", "Minneapolis North Loop", "Central", "Retail", "Minneapolis", "MN", "taylor.brooks", "2016-08-27", 16000),
    ("S06", "Kansas City Outlet", "Central", "Outlet", "Kansas City", "MO", "alex.romero", "2019-02-23", 11000),
    ("S07", "Boston Back Bay", "East", "Retail", "Boston", "MA", "jordan.lee", "2011-10-01", 21000),
    ("S08", "Burlington Lakeside", "East", "Retail", "Burlington", "VT", "sam.patel", "2018-06-09", 9000),
    ("S09", "Philadelphia Outlet", "East", "Outlet", "Philadelphia", "PA", "drew.kowalski", "2020-11-14", 13000),
    ("S10", "Asheville River Arts", "South", "Retail", "Asheville", "NC", "avery.james", "2015-04-25", 10000),
    ("S11", "Austin South Congress", "South", "Retail", "Austin", "TX", "parker.diaz", "2016-03-05", 19000),
    ("S12", "Atlanta Outlet", "South", "Outlet", "Atlanta", "GA", "quinn.foster", "2021-09-18", 14000),
    ("S13", "Salt Lake City Sugar House", "West", "Retail", "Salt Lake City", "UT", "riley.chen", "2022-05-07", 15000),
    ("S14", "Acme Web Store", "Online", "Online", "", "", "rowan.blake", "2010-01-15", None),
    ("S15", "Acme Mobile App", "Online", "Online", "", "", "emery.walsh", "2019-07-01", None),
]
STORE_WEIGHTS = [7, 5, 4, 6, 5, 4, 6, 3, 4, 3, 5, 4, 4, 18, 9]
EMAIL_DOMAIN = "acmeoutfitters.example"
DYNAMIC_RLS_USER = f"riley.chen@{EMAIL_DOMAIN}"

# (category, subcategory, price range, product nouns)
CATALOG = [
    ("Camping", "Tents", (149, 549), ["2P Tent", "4P Tent", "Ultralight Tent", "Dome Tent", "Tarp Shelter", "Bivy", "Family Tent", "Footprint"]),
    ("Camping", "Sleeping Bags", (79, 329), ["20F Bag", "35F Bag", "Quilt", "Down Bag", "Synthetic Bag", "Kids Bag", "Bag Liner", "Double Bag"]),
    ("Camping", "Camp Kitchen", (19, 129), ["Stove", "Cookset Kit", "Mug", "Spork Set", "Water Filter", "Cooler", "Fuel Canister", "Dish Kit"]),
    ("Hiking", "Footwear", (89, 229), ["Trail Runner", "Mid Boot", "Hiking Boot", "Approach Shoe", "Sandal", "Gaiters", "Wool Socks", "Insoles"]),
    ("Hiking", "Backpacks", (59, 289), ["Daypack 20L", "Daypack 28L", "Pack 45L", "Pack 65L", "Hydration Pack", "Hip Pack", "Kids Pack", "Rain Cover"]),
    ("Hiking", "Trekking Poles", (29, 149), ["Carbon Poles", "Aluminum Poles", "Folding Poles", "Trekking Staff", "Pole Tips", "Snow Baskets", "Cork Poles", "Kids Poles"]),
    ("Climbing", "Harnesses", (49, 139), ["Sport Harness", "Alpine Harness", "Kids Harness", "Big Wall Harness", "Gym Harness", "Chalk Bag", "Belay Device", "Locking Carabiner"]),
    ("Climbing", "Ropes", (99, 299), ["9.5mm Rope 60m", "9.8mm Rope 70m", "Static Rope", "Rope Bag", "Cordelette", "Sling Set", "Quickdraw Set", "Rope Brush"]),
    ("Climbing", "Helmets", (49, 129), ["Climbing Helmet", "Alpine Helmet", "Kids Helmet", "Headlamp", "Helmet Clip", "Ice Axe", "Crampons", "Avalanche Kit"]),
    ("Apparel", "Jackets", (79, 399), ["Rain Jacket", "Down Jacket", "Softshell", "Insulated Vest", "Wind Shell", "Fleece", "Parka", "Anorak"]),
    ("Apparel", "Base Layers", (29, 99), ["Merino Crew", "Merino Bottom", "Synthetic Tee", "Sun Hoodie", "Liner Gloves", "Neck Gaiter", "Long Sleeve", "Base Set Kit"]),
    ("Apparel", "Headwear", (15, 45), ["Beanie", "Sun Hat", "Trucker Cap", "Balaclava", "Bucket Hat", "Headband", "Visor", "Brim Hat"]),
    ("Paddling", "Kayaks", (399, 1299), ["Touring Kayak", "Sit-On-Top Kayak", "Inflatable Kayak", "Tandem Kayak", "Fishing Kayak", "Kids Kayak", "Packraft", "SUP Board"]),
    ("Paddling", "Paddles", (59, 249), ["Carbon Paddle", "Fiberglass Paddle", "Kids Paddle", "SUP Paddle", "Canoe Paddle", "Breakdown Paddle", "Paddle Leash", "Drip Rings"]),
    ("Paddling", "Life Vests", (49, 149), ["Touring PFD", "Kids PFD", "Inflatable PFD", "Fishing PFD", "Dog PFD", "Rescue PFD", "Whistle Kit", "Dry Bag"]),
]
BRANDS = ["Summit", "Ridge", "Cascade", "Juniper", "Granite", "Tamarack", "Basalt", "Alder"]
RETURN_REASONS = ["Wrong Size", "Damaged In Transit", "Changed Mind", "Not As Described"]
DEVICES = ["Desktop", "Mobile", "Tablet"]
TRAFFIC = ["Organic Search", "Paid Search", "Email Campaign", "Social Campaign", "Direct", "Referral"]
MONTH_WEIGHTS = [6, 6, 7, 8, 9, 9, 9, 8, 8, 8, 10, 12]
REGIONS = ["West", "Central", "East", "South", "Online"]


def noisy_case(rng: random.Random, text: str) -> str:
    """A whitespace/casing variant that the M cleaning steps normalize."""
    return rng.choice([text.lower() + " ", " " + text, text.upper(), text + "  "])


def generate_sources(rng: random.Random) -> dict:
    # Stores ------------------------------------------------------------------
    store_rows = []
    for sid, name, region, channel, city, state, mgr, opened, sqft in STORES:
        email = f"{mgr}@{EMAIL_DOMAIN}"
        if sid == "S02":
            email = "  Morgan.Price@AcmeOutfitters.example"
        if sid == "S13":
            region = region + " "
        if sid == "S09":
            sid = "s09 "
        store_rows.append([sid, name, region, channel, city, state, email, opened,
                           "" if sqft is None else str(sqft)])
    store_rows.insert(9, list(store_rows[6]))  # exact duplicate of S07

    # Products ----------------------------------------------------------------
    products = []
    n = 0
    for cat, sub, (lo, hi), nouns in CATALOG:
        for i, noun in enumerate(nouns):
            n += 1
            brand = BRANDS[(n * 3 + i) % len(BRANDS)]
            price = rng.randrange(lo // 5 + 1, hi // 5 + 1) * 5 - 1
            cost = money(D(str(price)) * D(f"{rng.uniform(0.42, 0.62):.2f}"))
            launch = date(2018, 1, 1) + timedelta(days=rng.randrange(0, 365 * 6))
            weight = rng.uniform(0.3, 3.0) * (0.35 if sub == "Kayaks" else 1.0)
            products.append({
                "id": f"P{n:04d}", "name": f"{brand} {noun}", "category": cat, "subcategory": sub,
                "price": D(f"{price}.00"), "cost": cost, "launch": launch, "weight": weight,
            })
    never_sold = set(rng.sample([p["id"] for p in products], 6))
    product_rows = []
    for p in products:
        pid, sub = p["id"], p["subcategory"]
        if pid == "P0042":
            pid = "p0042 "
        if p["id"] in ("P0017", "P0063", "P0101"):
            sub = sub + " "
        product_rows.append([pid, p["name"], p["category"], sub, f"{p['price']:.2f}",
                             f"{p['cost']:.2f}", iso(p["launch"])])
    product_rows.insert(30, list(product_rows[11]))
    product_rows.insert(77, list(product_rows[64]))

    # Customers ---------------------------------------------------------------
    customers = []
    customer_rows = []
    for i in range(1, 401):
        first, last = rng.choice(FIRST), rng.choice(LAST)
        cid = f"C{i:04d}"
        segment = rng.choices(["Consumer", "Club", "Corporate"], weights=[70, 20, 10])[0]
        domain = "corp.example" if segment == "Corporate" else rng.choice(EMAIL_DOMAINS)
        email = f"{first}.{last.replace(chr(39), '')}{i}@{domain}".lower()
        city, state = rng.choice(CUSTOMER_CITIES)
        joined = date(2018, 1, 1) + timedelta(days=rng.randrange(0, 2738))
        tier = rng.choices(["Bronze", "Silver", "Gold", ""], weights=[40, 28, 17, 15])[0]
        k = rng.choices([None, 0, 1, 2], weights=[10, 10, 60, 20])[0]
        managers = "" if k is None else repr(sorted(rng.sample(ACCOUNT_MANAGERS, k)))
        customers.append({"id": cid, "weight": rng.uniform(0.2, 3.0)})
        raw_id = " " + cid.lower() if i % 47 == 5 else cid
        raw_email = email.upper() if i % 31 == 7 else email
        raw_segment = segment + " " if i % 29 == 3 else segment
        customer_rows.append([raw_id, first, last, raw_email, raw_segment, city, state, iso(joined),
                              tier, managers])
    for src, dst in ((17, 120), (203, 260), (311, 330)):
        customer_rows.insert(dst, list(customer_rows[src]))

    # Orders ------------------------------------------------------------------
    sellable = [p for p in products if p["id"] not in never_sold]
    product_weights = [p["weight"] for p in sellable]
    customer_weights = [c["weight"] for c in customers]
    drafts = []
    for year, count in ((2024, 1750), (2025, 2150)):
        months = rng.choices(range(1, 13), weights=MONTH_WEIGHTS, k=count)
        for m in months:
            d = date(year, m, rng.randint(1, dim(year, m)))
            store = rng.choices(STORES, weights=STORE_WEIGHTS)[0][0]
            product = rng.choices(sellable, weights=product_weights)[0]
            customer = rng.choices(customers, weights=customer_weights)[0]["id"]
            if product["subcategory"] == "Kayaks":
                qty = 1
            else:
                qty = rng.choices([1, 2, 3, 4, 5, 6], weights=[60, 25, 9, 3, 2, 1])[0]
            disc = rng.choices(["0", "0.05", "0.1", "0.15", "0.2", ""], weights=[52, 10, 15, 10, 8, 5])[0]
            if d >= date(2025, 12, 22):
                status, ship = "Pending", None
            elif rng.random() < 0.07:
                status, ship = "Cancelled", None
            else:
                status, ship = "Completed", d + timedelta(days=rng.randint(1, 6))
            drafts.append([d, ship, customer, store, product["id"], qty, product["price"], disc, status])
    # Unmatched keys: customers and a product missing from the dimension files.
    for k, (store, cust) in enumerate((("S01", "C0999"), ("S07", "C0998"), ("S14", "C0999"),
                                       ("S11", "C0998"), ("S15", "C0997"), ("S14", "C0998"))):
        drafts[100 + k * 577][2] = cust
        drafts[100 + k * 577][3] = store
    for k, store in enumerate(("S04", "S14", "S10", "S15")):
        row = drafts[250 + k * 811]
        row[3], row[4], row[5], row[6] = store, "P9999", 1, D("59.00")
    drafts.sort(key=lambda r: r[0])

    orders = []
    order_rows = []
    for i, (d, ship, cust, store, pid, qty, price, disc, status) in enumerate(drafts, start=1):
        oid = f"SO-{100000 + i}"
        orders.append({"id": oid, "date": d, "ship": ship, "status": status, "qty": qty,
                       "price": price, "disc": disc})
        raw_pid = pid.lower() if i % 53 == 11 else pid
        raw_cust = " " + cust.lower() if i % 97 == 13 else cust
        raw_store = store.lower() + " " if i % 89 == 21 else store
        raw_status = noisy_case(rng, status) if i % 25 == 4 else status
        order_rows.append([oid, iso(d), iso(ship), raw_cust, raw_store, raw_pid, str(qty),
                           f"{price:.2f}", disc, raw_status])
    # Exact and near duplicates (removed by Table.Distinct) and rows without an id.
    for _ in range(12):
        src = rng.randrange(len(order_rows))
        order_rows.insert(rng.randrange(len(order_rows)), list(order_rows[src]))
    for _ in range(4):
        dup = list(order_rows[rng.randrange(len(order_rows))])
        dup[9] = noisy_case(rng, dup[9].strip().title())
        order_rows.insert(rng.randrange(len(order_rows)), dup)
    for _ in range(3):
        junk = list(order_rows[rng.randrange(len(order_rows))])
        junk[0] = rng.choice(["", " "])
        order_rows.insert(rng.randrange(len(order_rows)), junk)

    # Returns (at most one per order) ---------------------------------------------
    return_rows = []
    r = 0
    for o in orders:
        if o["status"] != "Completed" or rng.random() >= 0.045:
            continue
        r += 1
        qret = rng.randint(1, o["qty"])
        refund = money(o["price"] * (1 - D(o["disc"] or "0")) * qret)
        when = o["ship"] + timedelta(days=rng.randint(3, 30))
        reason = rng.choice(RETURN_REASONS)
        if r % 9 == 4:
            reason = noisy_case(rng, reason)
        return_rows.append([f"R{r:05d}", o["id"], iso(when), str(qret), reason, f"{refund:.2f}"])
    return_rows.insert(40, list(return_rows[12]))

    # Web sessions (no relationship to anything) ------------------------------------
    session_rows = []
    for i in range(1, 1601):
        d = CAL_START + timedelta(days=rng.randrange(len(CALENDAR)))
        device = rng.choices(DEVICES, weights=[45, 45, 10])[0]
        source = rng.choices(TRAFFIC, weights=[30, 15, 12, 13, 20, 10])[0]
        pages = rng.randint(1, 15)
        p_conv = (0.03 + (0.04 if "Campaign" in source else 0)
                  + (0.02 if device == "Desktop" else 0) + pages * 0.004)
        converted = "true" if rng.random() < p_conv else "false"
        raw_device = noisy_case(rng, device) if i % 37 == 9 else device
        session_rows.append([f"WS{i:06d}", iso(d), raw_device, source, str(pages), converted])

    return {
        "stores": store_rows,
        "products": product_rows,
        "customers": customer_rows,
        "orders": order_rows,
        "returns": return_rows,
        "web_sessions": session_rows,
        "never_sold": sorted(never_sold),
    }


def make_targets(rng: random.Random, orders: list[dict]) -> list[list[str]]:
    """Monthly targets per region, near the actual non-cancelled sales."""
    actual: dict[tuple[str, int, int], Decimal] = {}
    for o in orders:
        if o["Status"] == "Cancelled":
            continue
        d = o["Order Date"]
        key = (o["_store"]["Region"], d.year, d.month)
        actual[key] = actual.get(key, Decimal(0)) + o["Line Amount"]
    rows = []
    for region in REGIONS:
        for y in (2024, 2025):
            for m in range(1, 13):
                a = float(actual.get((region, y, m), Decimal(0)))
                target = int(round(a * rng.uniform(0.92, 1.12) / 500.0)) * 500
                raw_region = region + " " if (m + y) % 7 == 0 else region
                rows.append([raw_region, f"{y}-{m:02d}", str(target)])
    rows.insert(57, ["", "", ""])
    return rows


HEADERS = {
    "orders": ["order_id", "order_date", "ship_date", "customer_id", "store_id", "product_id",
               "quantity", "unit_price", "discount", "status"],
    "customers": ["customer_id", "first_name", "last_name", "email", "segment", "city", "state",
                  "join_date", "loyalty_tier", "account_managers"],
    "products": ["product_id", "product_name", "category", "subcategory", "list_price",
                 "unit_cost", "launch_date"],
    "stores": ["store_id", "store_name", "region", "channel", "city", "state", "manager_email",
               "open_date", "square_feet"],
    "targets": ["region", "month", "target_amount"],
    "returns": ["return_id", "order_id", "return_date", "quantity_returned", "reason",
                "refund_amount"],
    "web_sessions": ["session_id", "session_date", "device", "traffic_source", "pages_viewed",
                     "converted"],
}


# ---------------------------------------------------------------------------
# Python replicas of the M cleaning steps
# ---------------------------------------------------------------------------


def clean_key(x: str) -> str:
    return x.strip().upper()


def proper(x: str) -> str:
    return x.strip().title()


def parse_names(raw: str) -> str:
    """fnParseNames: a Python-repr list of names -> "A; B" ("" when empty)."""
    t = raw.strip()
    inner = t[1:len(t) - 1] if t[:1] == "[" else t
    names = [p.strip().strip("'\"") for p in inner.split(",")]
    names = [n for n in names if n != ""]
    return "; ".join(names)


def opt_date(text: str) -> date | None:
    return date.fromisoformat(text) if text.strip() else None


def distinct(rows: list[tuple]) -> list[tuple]:
    seen: set[tuple] = set()
    out = []
    for r in rows:
        if r not in seen:
            seen.add(r)
            out.append(r)
    return out


def clean_tables(raw: dict) -> dict:
    stores = []
    for r in distinct([(clean_key(r[0]), r[1], r[2].strip(), r[3], r[4], r[5], r[6].strip().lower(),
                        r[7], r[8]) for r in raw["stores"]]):
        sid, name, region, channel, city, state, email, opened, sqft = r
        stores.append({
            "Store ID": sid, "Store Name": name, "Region": region, "Channel": channel,
            "City": city or None, "State": state or None, "Manager Email": email,
            "Store Manager": email.split("@")[0].replace(".", " ").title(),
            "Region Code": region[:3].upper(), "Open Date": date.fromisoformat(opened),
            "Square Feet": int(sqft) if sqft else None,
        })
    for s in stores:
        s["Store Label"] = f"{s['Store Name']} ({s['Region']})"

    products = []
    for r in distinct([(clean_key(r[0]), r[1].strip(), r[2].strip(), r[3].strip(), r[4], r[5], r[6])
                       for r in raw["products"]]):
        pid, name, cat, sub, price, cost, launch = r
        lp = D(price)
        products.append({
            "Product ID": pid, "Product Name": name, "Category": cat, "Subcategory": sub,
            "List Price": lp, "Unit Cost": D(cost), "Launch Date": date.fromisoformat(launch),
            "Price Band": "Under $50" if lp < 50 else ("$50 to $149" if lp < 150 else "$150 and up"),
        })

    customers = []
    for r in distinct([(clean_key(r[0]), r[1], r[2], r[3].strip().lower(), r[4].strip(), r[5], r[6], r[7],
                        r[8].strip() or "Unassigned", parse_names(r[9]) or None)
                       for r in raw["customers"]]):
        cid, first, last, email, segment, city, state, joined, tier, managers = r
        customers.append({
            "Customer ID": cid, "Customer Name": f"{first.strip()} {last.strip()}", "Email": email,
            "Email Domain": email.split("@", 1)[1], "Segment": segment, "City": city, "State": state,
            "Join Date": date.fromisoformat(joined), "Loyalty Tier": tier,
            "Account Managers": managers,
            "Account Manager Count": len([s for s in (managers or "").split(";") if s.strip()]),
        })

    store_by = {s["Store ID"]: s for s in stores}
    product_by = {p["Product ID"]: p for p in products}
    customer_by = {c["Customer ID"]: c for c in customers}

    orders = []
    rows = [r for r in raw["orders"] if r[0].strip() != ""]
    for r in distinct([(r[0].strip(), r[1], r[2], clean_key(r[3]), clean_key(r[4]), clean_key(r[5]),
                        r[6], r[7], r[8], proper(r[9])) for r in rows]):
        oid, od, sd, cid, sid, pid, qty, price, disc, status = r
        q, p, dc = int(qty), D(price), D(disc) if disc.strip() else Decimal(0)
        product = product_by.get(pid)
        orders.append({
            "Order ID": oid, "Order Date": date.fromisoformat(od), "Ship Date": opt_date(sd),
            "Customer ID": cid, "Store ID": sid, "Product ID": pid, "Quantity": q,
            "Unit Price": p, "Discount": dc, "Status": status, "Line Amount": q * p * (1 - dc),
            "Line Cost": q * product["Unit Cost"] if product else None,
            "_store": store_by[sid], "_product": product, "_customer": customer_by.get(cid),
        })
    order_by = {o["Order ID"]: o for o in orders}
    assert len(order_by) == len(orders), "order ids must be unique after cleaning"

    returns = []
    for r in distinct([(r[0].strip(), r[1].strip(), r[2], r[3], proper(r[4]), r[5]) for r in raw["returns"]]):
        rid, oid, rd, qret, reason, refund = r
        returns.append({"Return ID": rid, "Order ID": oid, "Return Date": date.fromisoformat(rd),
                        "Quantity Returned": int(qret), "Reason": reason, "Refund Amount": D(refund),
                        "_order": order_by[oid]})
    assert len({x["Order ID"] for x in returns}) == len(returns), "one return per order (1:1)"
    return_by_order = {x["Order ID"]: x for x in returns}

    sessions = []
    for r in distinct([(r[0], r[1], proper(r[2]), r[3], r[4], r[5]) for r in raw["web_sessions"]]):
        sid, sd, device, source, pages, conv = r
        sessions.append({"Session ID": sid, "Session Date": date.fromisoformat(sd), "Device": device,
                         "Traffic Source": source, "Pages Viewed": int(pages),
                         "Converted": conv == "true", "Is Campaign": "Campaign" in source})

    # Calculated columns (DAX) ---------------------------------------------------
    by_customer: dict[str, list[dict]] = {}
    by_product: dict[str, list[dict]] = {}
    for o in orders:
        by_customer.setdefault(o["Customer ID"], []).append(o)
        by_product.setdefault(o["Product ID"], []).append(o)
        amount = o["Line Amount"]
        o["Order Size"] = "Large" if amount >= 1000 else ("Medium" if amount >= 250 else "Small")
        o["Is Returned"] = "Yes" if o["Order ID"] in return_by_order else "No"
        o["Days To Ship"] = (o["Ship Date"] - o["Order Date"]).days if o["Ship Date"] else None
        o["_return"] = return_by_order.get(o["Order ID"])
    for c in customers:
        mine = by_customer.get(c["Customer ID"], [])
        c["Lifetime Sales"] = sum((o["Line Amount"] for o in mine), Decimal(0)) if mine else None
        c["Has Orders"] = "Yes" if mine else "No"
        ls = c["Lifetime Sales"]
        c["Value Tier"] = ("High" if ls is not None and ls >= 3000 else
                           "Medium" if ls is not None and ls >= 1000 else
                           "Low" if ls is not None and ls > 0 else "None")
    for p in products:
        mine = by_product.get(p["Product ID"], [])
        p["Units Sold (All Time)"] = sum(o["Quantity"] for o in mine) if mine else None

    return {
        "Stores": stores, "Products": products, "Customers": customers, "Orders": orders,
        "Returns": returns, "Web Sessions": sessions,
    }


def clean_targets(rows: list[list[str]]) -> list[dict]:
    out = []
    for r in rows:
        if r[0].strip() == "":
            continue
        y, m = int(r[1][:4]), int(r[1][5:7])
        out.append({"Region": r[0].strip(), "Month": date(y, m, 1), "Target Amount": D(r[2])})
    return out


# ---------------------------------------------------------------------------
# Independent measure engine (plain Python over the cleaned rows)
# ---------------------------------------------------------------------------

MONTH_ABBR = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]


def date_row(d: date) -> dict:
    return {
        "Date": d, "Year": d.year, "Month Number": d.month, "Month": MONTH_ABBR[d.month - 1],
        "Year Month": f"{d.year}-{d.month:02d}", "Quarter": f"Q{(d.month - 1) // 3 + 1}",
        "Month Start": date(d.year, d.month, 1),
        "Day Type": "Weekend" if d.weekday() >= 5 else "Weekday",
    }


DATE_ROWS = {d: date_row(d) for d in CALENDAR}
BLANK_ROWS = {"Stores": {}, "Products": {}, "Customers": {}}


class Ctx:
    """A filter context: per-table column predicates, an optional explicit date set
    (time intelligence), a calculation item and USERELATIONSHIP on the ship date."""

    def __init__(self, preds=None, dates=None, item=None, ship=False):
        self.preds: dict[str, tuple] = preds or {}
        self.dates: frozenset | None = dates
        self.item: str | None = item
        self.ship = ship

    def replace(self, **kw) -> "Ctx":
        c = Ctx(self.preds, self.dates, self.item, self.ship)
        for k, v in kw.items():
            setattr(c, k, v)
        return c

    def add(self, table: str, column: str, fn) -> "Ctx":
        p = dict(self.preds)
        p[table] = p.get(table, ()) + ((column, fn),)
        return self.replace(preds=p)

    def without(self, table: str, column: str | None = None) -> "Ctx":
        p = dict(self.preds)
        if column is None:
            p.pop(table, None)
        else:
            p[table] = tuple(x for x in p.get(table, ()) if x[0] != column)
        return self.replace(preds=p)

    def ok(self, table: str, row: dict) -> bool:
        return all(fn(row) for _, fn in self.preds.get(table, ()))


def dsum(values) -> Decimal | None:
    vals = [v for v in values if v is not None]
    return sum(vals, Decimal(0)) if vals else None


def sub(a, b):
    if a is None and b is None:
        return None
    return Decimal(a or 0) - Decimal(b or 0)


def divide(a, b):
    if a is None or b is None or b == 0:
        return None
    return Decimal(a) / Decimal(b)


def count(rows) -> int | None:
    n = len(rows)
    return n or None


def shift_dates(dates, months: int) -> frozenset:
    """DATEADD / SAMEPERIODLASTYEAR: a month's last day extends to the end of the
    target month; dates outside the calendar are dropped."""
    out = set()
    for d in dates:
        y, m = divmod(d.year * 12 + d.month - 1 + months, 12)
        m += 1
        last = dim(y, m)
        nd = date(y, m, min(d.day, last))
        out.add(nd)
        if d.day == dim(d.year, d.month):
            out.update(date(y, m, day) for day in range(nd.day, last + 1))
    return frozenset(x for x in out if x in CAL_SET)


class Engine:
    def __init__(self, tables: dict, targets: list[dict], never_sold: list[str]):
        self.t = tables
        self.targets_rows = targets
        self.never_sold = never_sold

    # Visible rows ---------------------------------------------------------------
    def visible_dates(self, ctx: Ctx):
        if ctx.dates is not None:
            return ctx.dates
        if not ctx.preds.get("Date"):
            return None
        return frozenset(d for d in CALENDAR if ctx.ok("Date", DATE_ROWS[d]))

    def base_dates(self, ctx: Ctx) -> frozenset:
        v = self.visible_dates(ctx)
        return CAL_SET if v is None else v

    def orders(self, ctx: Ctx) -> list[dict]:
        dates = self.visible_dates(ctx)
        out = []
        for o in self.t["Orders"]:
            if not ctx.ok("Stores", o["_store"]):
                continue
            if not ctx.ok("Products", o["_product"] or BLANK_ROWS["Products"]):
                continue
            if not ctx.ok("Customers", o["_customer"] or BLANK_ROWS["Customers"]):
                continue
            if dates is not None:
                key = o["Ship Date"] if ctx.ship else o["Order Date"]
                if key is None or key not in dates:
                    continue
            if not ctx.ok("Orders", o):
                continue
            if ctx.preds.get("Returns") and not (o["_return"] and ctx.ok("Returns", o["_return"])):
                continue  # one-to-one, both directions: a filter on Returns filters Orders
            out.append(o)
        return out

    def orders_filtered(self, ctx: Ctx) -> bool:
        return ctx.dates is not None or any(ctx.preds.get(t) for t in ("Stores", "Products", "Orders", "Date",
                                                                      "Returns"))

    def customers(self, ctx: Ctx) -> list[dict]:
        rows = [c for c in self.t["Customers"] if ctx.ok("Customers", c)]
        if self.orders_filtered(ctx):  # bidirectional Orders -> Customers
            ids = {o["Customer ID"] for o in self.orders(ctx)}
            rows = [c for c in rows if c["Customer ID"] in ids]
        return rows

    def returns(self, ctx: Ctx) -> list[dict]:
        ids = {o["Order ID"] for o in self.orders(ctx)}
        return [r for r in self.t["Returns"] if r["Order ID"] in ids and ctx.ok("Returns", r)]

    def targets(self, ctx: Ctx) -> list[dict]:
        regions = None
        if ctx.preds.get("Stores"):  # many-to-many on Region, Stores filters Targets
            regions = {s["Region"] for s in self.t["Stores"] if ctx.ok("Stores", s)}
        dates = self.visible_dates(ctx)
        return [t for t in self.targets_rows
                if (regions is None or t["Region"] in regions)
                and (dates is None or t["Month"] in dates)]

    def sessions(self, ctx: Ctx) -> list[dict]:
        return [s for s in self.t["Web Sessions"] if ctx.ok("Web Sessions", s)]

    # Measures ----------------------------------------------------------------------
    def measure(self, name: str, ctx: Ctx):
        fn = MEASURES[name][0]
        item = ctx.item
        if item in (None, "Current"):
            return fn(self, ctx.replace(item=None))
        base = ctx.replace(item=None)
        if item == "YTD":
            return fn(self, self.ytd(base))
        if item == "PY":
            return fn(self, self.py(base))
        if item == "YoY %":
            cur, prev = fn(self, base), fn(self, self.py(base))
            return divide(sub(cur, prev), prev)
        raise ValueError(item)

    def format_of(self, name: str, ctx: Ctx, value) -> str:
        if ctx.item == "YoY %":
            return "+0.0%;-0.0%;0.0%"
        dynamic = MEASURES[name][2]
        return dynamic(value) if dynamic else MEASURES[name][1]

    def value(self, name: str, ctx: Ctx) -> dict:
        v = self.measure(name, ctx)
        return {"value": to_json(v), "formatted": fmt(v, self.format_of(name, ctx, v))}

    def ytd(self, ctx: Ctx) -> Ctx:
        dates = self.base_dates(ctx)
        last = max(dates) if dates else None
        ytd = frozenset(d for d in CALENDAR if last and d.year == last.year and d <= last)
        return ctx.replace(dates=ytd)

    def py(self, ctx: Ctx) -> Ctx:
        return ctx.replace(dates=shift_dates(self.base_dates(ctx), -12))

    def prev_month(self, ctx: Ctx) -> Ctx:
        return ctx.replace(dates=shift_dates(self.base_dates(ctx), -1))


def _ts(e: Engine, c: Ctx):
    return dsum(o["Line Amount"] for o in e.orders(c))


def _tc(e: Engine, c: Ctx):
    return dsum(o["Line Cost"] for o in e.orders(c))


def _target(e: Engine, c: Ctx):
    return dsum(t["Target Amount"] for t in e.targets(c))


def _sessions(e: Engine, c: Ctx):
    return count(e.sessions(c))


def scaled_format(v) -> str:
    if v is not None and abs(v) >= 1000000:
        return "\\$#,0.00,,\\M"
    if v is not None and abs(v) >= 1000:
        return "\\$#,0.0,\\K"
    return "\\$#,0.00"


# name -> (python fn, format string, dynamic format fn)
MEASURES = {
    "Total Sales": (_ts, "\\$#,0.00", None),
    "Total Cost": (_tc, "\\$#,0.00", None),
    "Gross Margin": (lambda e, c: sub(_ts(e, c), _tc(e, c)), "\\$#,0.00", None),
    "Margin %": (lambda e, c: divide(sub(_ts(e, c), _tc(e, c)), _ts(e, c)), "0.0%", None),
    "Sales (Scaled)": (_ts, "\\$#,0.00", scaled_format),
    "Completed Sales": (lambda e, c: _ts(e, c.add("Orders", "Status", lambda o: o["Status"] == "Completed")), "\\$#,0.00", None),
    "Shipped Sales": (lambda e, c: _ts(e, c.replace(ship=True)), "\\$#,0.00", None),
    "Sales All Stores": (lambda e, c: _ts(e, c.without("Stores")), "\\$#,0.00", None),
    "Store Share": (lambda e, c: divide(_ts(e, c), _ts(e, c.without("Stores"))), "0.0%", None),
    "Order Count": (lambda e, c: count(e.orders(c)), "#,0", None),
    "Units Sold": (lambda e, c: (sum(o["Quantity"] for o in e.orders(c)) or None), "#,0", None),
    "Active Customers": (lambda e, c: count({o["Customer ID"] for o in e.orders(c) if o["Customer ID"]}), "#,0", None),
    "Avg Order Value": (lambda e, c: divide(_ts(e, c), count(e.orders(c))), "\\$#,0.00", None),
    "Revenue per Unit": (lambda e, c: divide(_ts(e, c), sum(o["Quantity"] for o in e.orders(c)) or None), "\\$#,0.00", None),
    "Last Order Date": (lambda e, c: max((o["Order Date"] for o in e.orders(c)), default=None), "yyyy-mm-dd", None),
    "Unknown Product Orders": (lambda e, c: len([o for o in e.orders(c) if o["_product"] is None]), "0", None),
    "Sales YTD": (lambda e, c: _ts(e, e.ytd(c)), "\\$#,0.00", None),
    "Sales PY": (lambda e, c: _ts(e, e.py(c)), "\\$#,0.00", None),
    "Sales YoY %": (lambda e, c: divide(sub(_ts(e, c), _ts(e, e.py(c))), _ts(e, e.py(c))), "+0.0%;-0.0%;0.0%", None),
    "Sales Prev Month": (lambda e, c: _ts(e, e.prev_month(c)), "\\$#,0.00", None),
    "Customer Count": (lambda e, c: count(e.customers(c)), "#,0", None),
    "Products Never Sold Count": (lambda e, c: len(e.never_sold), "0", None),
    "Returns Count": (lambda e, c: count(e.returns(c)), "#,0", None),
    "Refunds": (lambda e, c: dsum(r["Refund Amount"] for r in e.returns(c)), "\\$#,0.00", None),
    "Return Rate": (lambda e, c: divide(count(e.returns(c)), count(e.orders(c))), "0.0%", None),
    "Target": (_target, "\\$#,0", None),
    "Target Attainment %": (lambda e, c: divide(_ts(e, c), _target(e, c)), "0.0%", None),
    "Target Stretch": (lambda e, c: None if _target(e, c) is None else _target(e, c) * D("1.2"), "\\$#,0", None),
    "Sessions": (_sessions, "#,0", None),
    "Converted Sessions": (lambda e, c: count([s for s in e.sessions(c) if s["Converted"]]), "#,0", None),
    "Conversion Rate": (lambda e, c: divide(count([s for s in e.sessions(c) if s["Converted"]]), _sessions(e, c)), "0.0%", None),
    "Avg Pages per Session": (lambda e, c: divide(sum(s["Pages Viewed"] for s in e.sessions(c)), _sessions(e, c)), "0.0", None),
    "Sessions (All Devices)": (lambda e, c: _sessions(e, c.without("Web Sessions", "Device")), "#,0", None),
}


# ---------------------------------------------------------------------------
# Value formatting (as Power BI formats with these format strings)
# ---------------------------------------------------------------------------

BOUNDARY_ISSUES: list[str] = []


def to_json(v):
    if v is None:
        return None
    if isinstance(v, date):
        return v.isoformat()
    if isinstance(v, bool):
        return v
    if isinstance(v, int):
        return v
    d = Decimal(v)
    if d == d.to_integral_value():
        return int(d)
    return float(round(d, 10))


def _split_sections(f: str) -> list[str]:
    out, cur, i, quoted = [], "", 0, False
    while i < len(f):
        c = f[i]
        if c == "\\" and i + 1 < len(f):
            cur += f[i:i + 2]
            i += 2
            continue
        if c == '"':
            quoted = not quoted
        if c == ";" and not quoted:
            out.append(cur)
            cur = ""
        else:
            cur += c
        i += 1
    out.append(cur)
    return out


def _group(digits: str) -> str:
    out = ""
    while len(digits) > 3:
        out = "," + digits[-3:] + out
        digits = digits[:-3]
    return digits + out


def fmt(v, f: str) -> str:
    if v is None:
        return "(Blank)"
    if isinstance(v, date):
        assert f == "yyyy-mm-dd", f
        return v.isoformat()
    v = Decimal(v)
    sections = _split_sections(f)
    sign = ""
    if v < 0 and len(sections) > 1:
        sec, v = sections[1], -v
    elif v == 0 and len(sections) > 2:
        sec = sections[2]
    else:
        sec = sections[0]
        if v < 0:
            sign, v = "-", -v
    prefix, suffix, num = "", "", ""
    state, i, percent = "pre", 0, False
    while i < len(sec):
        c = sec[i]
        if c == "\\":
            lit, i = sec[i + 1], i + 2
        elif c in "#0,." and state != "post":
            num += c
            state, i = "num", i + 1
            continue
        else:
            lit, i = c, i + 1
            percent = percent or c == "%"
        if state == "pre":
            prefix += lit
        else:
            state = "post"
            suffix += lit
    scale = 0
    while num.endswith(","):
        num, scale = num[:-1], scale + 1
    int_part, _, frac_part = num.partition(".")
    decimals = len(frac_part)
    x = v * (100 if percent else 1) / (Decimal(1000) ** scale)
    q = Decimal(1).scaleb(-decimals)
    shifted = abs(x) * (Decimal(10) ** decimals)
    frac = shifted - shifted.to_integral_value(rounding="ROUND_FLOOR")
    if abs(frac - Decimal("0.5")) < Decimal("0.000001"):
        BOUNDARY_ISSUES.append(f"{v} with {f}")
    r = x.quantize(q, rounding=ROUND_HALF_UP)
    text = f"{r:f}"
    whole, _, dec = text.partition(".")
    if "," in int_part:
        whole = _group(whole)
    out = whole + ("." + dec if decimals else "")
    return sign + prefix + out + suffix


# ---------------------------------------------------------------------------
# PBIR building blocks: semantic-query expressions, filters, visuals
# ---------------------------------------------------------------------------

SCHEMA = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition"
MEASURE_HOME: dict[str, str] = {}  # measure -> table, filled from the model definition


def lit(v) -> str:
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return f"{v}L"
    if isinstance(v, (float, Decimal)):
        return f"{v}D"
    if isinstance(v, date):
        return f"datetime'{v.isoformat()}T00:00:00'"
    if v is None:
        return "null"
    return "'" + str(v).replace("'", "''") + "'"


def L(v) -> dict:
    return {"expr": {"Literal": {"Value": lit(v)}}}


def color(hex_color: str) -> dict:
    return {"solid": {"color": L(hex_color)}}


def col(table: str, column: str) -> dict:
    return {"Column": {"Expression": {"SourceRef": {"Entity": table}}, "Property": column}}


def meas(name: str) -> dict:
    return {"Measure": {"Expression": {"SourceRef": {"Entity": MEASURE_HOME[name]}}, "Property": name}}


def hlevel(table: str, hierarchy: str, level: str) -> dict:
    return {"HierarchyLevel": {"Expression": {"Hierarchy": {
        "Expression": {"SourceRef": {"Entity": table}}, "Hierarchy": hierarchy}}, "Level": level}}


def alias_of(table: str) -> str:
    return table[0].lower()


def acol(alias: str, column: str) -> dict:
    return {"Column": {"Expression": {"SourceRef": {"Source": alias}}, "Property": column}}


def C(table: str, column: str) -> tuple:
    return ("col", table, column)


def M(name: str) -> tuple:
    return ("m", name)


def H(table: str, hierarchy: str, level: str) -> tuple:
    return ("h", table, hierarchy, level)


def field_json(f: tuple) -> dict:
    if f[0] == "col":
        return col(f[1], f[2])
    if f[0] == "m":
        return meas(f[1])
    return hlevel(f[1], f[2], f[3])


def query_ref(f: tuple) -> str:
    if f[0] == "col":
        return f"{f[1]}.{f[2]}"
    if f[0] == "m":
        return f"{MEASURE_HOME[f[1]]}.{f[1]}"
    return f"{f[1]}.{f[2]}.{f[3]}"


def native_ref(f: tuple) -> str:
    return f[-1] if f[0] != "m" else f[1]


def projection(f: tuple, display: str | None = None) -> dict:
    p = {"field": field_json(f), "queryRef": query_ref(f), "nativeQueryRef": native_ref(f)}
    if display:
        p["displayName"] = display
    return p


class F:
    """A column filter, as PBIR JSON and as a Python predicate."""

    def __init__(self, table: str, column: str, op: str, values=(), lo=None, hi=None,
                 hi_inclusive: bool = True, years: int | None = None):
        self.table, self.column, self.op = table, column, op
        self.values, self.lo, self.hi = tuple(values), lo, hi
        self.hi_inclusive, self.years = hi_inclusive, years

    @property
    def alias(self) -> str:
        return alias_of(self.table)

    def condition(self) -> dict:
        c = acol(self.alias, self.column)
        if self.op in ("in", "notin"):
            cond = {"In": {"Expressions": [c], "Values": [[{"Literal": {"Value": lit(v)}}] for v in self.values]}}
            return cond if self.op == "in" else {"Not": {"Expression": cond}}
        if self.op == "range":
            left = {"Comparison": {"ComparisonKind": 2, "Left": c, "Right": {"Literal": {"Value": lit(self.lo)}}}}
            right = {"Comparison": {"ComparisonKind": 4 if self.hi_inclusive else 3, "Left": c,
                                    "Right": {"Literal": {"Value": lit(self.hi)}}}}
            return {"And": {"Left": left, "Right": right}}
        if self.op == "relative":
            now = {"Now": {}}
            lower = {"DateSpan": {"Expression": {"DateAdd": {"Expression": {"DateAdd": {
                "Expression": now, "Amount": 1, "TimeUnit": 0}}, "Amount": -self.years, "TimeUnit": 3}},
                "TimeUnit": 0}}
            upper = {"DateSpan": {"Expression": now, "TimeUnit": 0}}
            return {"Between": {"Expression": c, "LowerBound": lower, "UpperBound": upper}}
        raise ValueError(self.op)

    def definition(self) -> dict:
        return {"Version": 2, "From": [{"Name": self.alias, "Entity": self.table, "Type": 0}],
                "Where": [{"Condition": self.condition()}]}

    def filter_type(self) -> str:
        return {"in": "Categorical", "notin": "Categorical", "range": "Advanced",
                "relative": "RelativeDate"}[self.op]

    def card(self, name: str, **extra) -> dict:
        return {"name": name, "field": col(self.table, self.column), "type": self.filter_type(),
                "filter": self.definition(), "howCreated": "User", **extra}

    def pred(self, row: dict) -> bool:
        v = row.get(self.column)
        if self.op == "in":
            return v in self.values
        if self.op == "notin":
            return v not in self.values
        if self.op == "range":
            if v is None:
                return False
            return self.lo <= v and (v <= self.hi if self.hi_inclusive else v < self.hi)
        return True  # relative dates: every row lies in the window (data ends 2025-12-31)

    def apply(self, ctx: Ctx) -> Ctx:
        if self.table == "Time Calc":
            assert self.op == "in" and len(self.values) == 1
            return ctx.replace(item=self.values[0])
        if self.op == "relative":
            return ctx
        return ctx.add(self.table, self.column, self.pred)

    def describe(self) -> str:
        ref = f"'{self.table}'[{self.column}]"
        values = "[" + ", ".join(f"'{v}'" if isinstance(v, str) else str(v) for v in self.values) + "]"
        if self.op == "in":
            return f"{ref} in {values}"
        if self.op == "notin":
            return f"{ref} not in {values}"
        if self.op == "range":
            return f"{ref} >= {self.lo} and {'<=' if self.hi_inclusive else '<'} {self.hi}"
        return f"{ref} in the last {self.years} years (relative date)"


class TopN:
    """A Top N visual filter."""

    def __init__(self, table: str, column: str, n: int, measure: str):
        self.table, self.column, self.n, self.measure = table, column, n, measure

    def card(self, name: str) -> dict:
        a = alias_of(self.table)
        home = MEASURE_HOME[self.measure]
        ma = alias_of(home) if home != self.table else a
        inner_from = [{"Name": a, "Entity": self.table, "Type": 0}]
        if ma != a:
            inner_from.append({"Name": ma, "Entity": home, "Type": 0})
        sub = {"Version": 2, "From": inner_from,
               "Select": [{**acol(a, self.column), "Name": f"{self.table}.{self.column}"}],
               "OrderBy": [{"Direction": 2, "Expression": {"Measure": {
                   "Expression": {"SourceRef": {"Source": ma}}, "Property": self.measure}}}],
               "Top": self.n}
        return {"name": name, "field": col(self.table, self.column), "type": "TopN",
                "filter": {"Version": 2, "From": [
                    {"Name": "subquery", "Expression": {"Subquery": {"Query": sub}}, "Type": 2},
                    {"Name": a, "Entity": self.table, "Type": 0}],
                    "Where": [{"Condition": {"In": {"Expressions": [acol(a, self.column)],
                                                    "Table": {"SourceRef": {"Source": "subquery"}}}}}]},
                "howCreated": "User"}

    def describe(self) -> str:
        return f"Top {self.n} '{self.table}'[{self.column}] by [{self.measure}]"


def vco(title: str | None = None, *, background: str | None = "#FFFFFF", border: bool = True,
        radius: int = 8, shadow: bool = False, tooltip_page: str | None = None) -> dict:
    o: dict = {}
    if title is not None:
        o["title"] = [{"properties": {"show": L(True), "text": L(title), "fontSize": L(11),
                                      "fontColor": color("#1F3A2E"), "bold": L(True)}}]
    else:
        o["title"] = [{"properties": {"show": L(False)}}]
    if background:
        o["background"] = [{"properties": {"show": L(True), "color": color(background),
                                           "transparency": L(0)}}]
    else:
        o["background"] = [{"properties": {"show": L(False)}}]
    o["border"] = [{"properties": {"show": L(border), "color": color("#D9D4C7"), "width": L(1),
                                   "radius": L(radius)}}]
    if shadow:
        o["dropShadow"] = [{"properties": {"show": L(True), "color": color("#7A7466"),
                                           "position": L("Outer"), "preset": L("BottomRight"),
                                           "transparency": L(70)}}]
    if tooltip_page:
        o["visualTooltip"] = [{"properties": {"show": L(True), "type": L("ReportPage"),
                                              "section": L(tooltip_page)}}]
    return o


def textbox_paragraphs(paragraphs: list[dict]) -> dict:
    return {"general": [{"properties": {"paragraphs": paragraphs}}]}


def run(text: str, *, size: str | None = None, bold: bool = False, color_hex: str | None = None,
        url: str | None = None, italic: bool = False) -> dict:
    style: dict = {}
    if size:
        style["fontSize"] = size
    if bold:
        style["fontWeight"] = "bold"
    if italic:
        style["fontStyle"] = "italic"
    if color_hex:
        style["color"] = color_hex
    r: dict = {"value": text}
    if url:
        r["url"] = url
    if style:
        r["textStyle"] = style
    return r


class Visual:
    def __init__(self, key: str, vid: str, vtype: str, rect, z: int):
        self.key, self.id, self.type, self.rect, self.z = key, vid, vtype, rect, z
        self.filters: list = []
        self.selection: F | None = None
        self.json: dict = {}


class Page:
    def __init__(self, report: "Report", key: str, display: str, width: int, height: int,
                 option: str, *, filters=(), visibility: str | None = None,
                 page_type: str | None = None):
        self.report, self.key, self.display = report, key, display
        self.id = hex20(report.key, "page", key)
        self.width, self.height, self.option = width, height, option
        self.filters: list[F] = list(filters)
        self.visibility, self.page_type = visibility, page_type
        self.visuals: list[Visual] = []
        self.interactions: list[tuple[str, str, str]] = []
        self.expect: list[dict] = []

    def v(self, key: str) -> Visual:
        return next(x for x in self.visuals if x.key == key)

    def add(self, key: str, vtype: str, rect, *, roles: dict | None = None, objects: dict | None = None,
            container: dict | None = None, filters=(), sort=None, selection: F | None = None,
            schema: str | None = None, hidden: bool = False) -> Visual:
        vis = Visual(key, hex20(self.report.key, self.key, key), vtype, rect, len(self.visuals) * 1000)
        vis.filters = list(filters)
        vis.selection = selection
        x, y, w, h = rect
        body: dict = {"visualType": vtype}
        if roles:
            body["query"] = {"queryState": {
                role: {"projections": [projection(*f) if isinstance(f, list) else projection(f) for f in fields]}
                for role, fields in roles.items()}}
            if sort:
                body["query"]["sortDefinition"] = {
                    "sort": [{"field": field_json(f), "direction": d} for f, d in sort],
                    "isDefaultSort": False}
        if objects:
            body["objects"] = objects
        body["visualContainerObjects"] = container if container is not None else vco()
        if roles:
            body["drillFilterOtherVisuals"] = True
        vjson = {
            "$schema": f"{SCHEMA}/visualContainer/{schema or self.report.visual_schema}/schema.json",
            "name": vis.id,
            "position": {"x": x, "y": y, "z": vis.z, "height": h, "width": w, "tabOrder": vis.z},
            "visual": body,
        }
        cards = []
        for i, f in enumerate(vis.filters):
            cards.append(f.card(hex20(vis.id, "filter", str(i))))
        if cards:
            vjson["filterConfig"] = {"filters": cards}
        if hidden:
            vjson["isHidden"] = True
        vis.json = vjson
        self.visuals.append(vis)
        return vis

    def interact(self, source: str, target: str, kind: str) -> None:
        self.interactions.append((source, target, kind))

    # Expectations ----------------------------------------------------------------
    def context_filters(self, target: str | None) -> list:
        out = list(self.report.filters) + list(self.filters)
        blocked = {s for s, t, k in self.interactions if t == target and k == "NoFilter"}
        for vis in self.visuals:
            if vis.selection is not None and vis.key not in blocked and vis.key != target:
                out.append(vis.selection)
        if target is not None:
            out += [f for f in self.v(target).filters if isinstance(f, F)]
        return out

    def ctx(self, target: str | None, extra=()) -> Ctx:
        c = Ctx()
        for f in self.context_filters(target) + list(extra):
            c = f.apply(c)
        return c

    def json(self) -> dict:
        page: dict = {
            "$schema": f"{SCHEMA}/page/2.1.0/schema.json",
            "name": self.id,
            "displayName": self.display,
            "displayOption": self.option,
            "height": self.height,
            "width": self.width,
        }
        if self.filters:
            page["filterConfig"] = {"filters": [f.card(hex20(self.id, "filter", str(i)))
                                                for i, f in enumerate(self.filters)]}
        if self.page_type == "Tooltip":
            page["pageBinding"] = {"name": hex20(self.id, "binding"), "type": "Tooltip", "parameters": []}
            page["type"] = "Tooltip"
        if self.visibility:
            page["visibility"] = self.visibility
        page["objects"] = {"background": [{"properties": {"color": color("#F7F5EF"), "transparency": L(0)}}]}
        if self.interactions:
            page["visualInteractions"] = [{"source": self.v(s).id, "target": self.v(t).id, "type": k}
                                          for s, t, k in self.interactions]
        return page


class Report:
    def __init__(self, key: str, filters, visual_schema: str):
        self.key = key
        self.name = REPORT_NAMES[key]
        self.filters: list[F] = list(filters)
        self.visual_schema = visual_schema
        self.pages: list[Page] = []
        self.bookmarks: list[dict] = []

    def page(self, *args, **kw) -> Page:
        p = Page(self, *args, **kw)
        self.pages.append(p)
        return p

    def page_by(self, key: str) -> Page:
        return next(p for p in self.pages if p.key == key)


# ---------------------------------------------------------------------------
# Semantic model (TMDL)
# ---------------------------------------------------------------------------


def tq(name: str) -> str:
    if name == "" or any(c.isspace() or c in ".=:'" for c in name):
        return "'" + name.replace("'", "''") + "'"
    return name


def tmdl_text(v: str) -> str:
    return v if v and v.strip() == v and '"' not in v else '"' + v.replace('"', '""') + '"'


class TmdlOut:
    def __init__(self):
        self.lines: list[str] = []

    def line(self, level: int, text: str) -> None:
        self.lines.append("\t" * level + text)

    def blank(self) -> None:
        self.lines.append("")

    def desc(self, level: int, text: str | None) -> None:
        if text:
            for part in text.split("\n"):
                self.line(level, f"/// {part}")

    def expr(self, level: int, head: str, expression: str) -> None:
        lines = expression.strip("\n").split("\n")
        if len(lines) == 1:
            self.line(level, f"{head} = {lines[0]}")
        else:
            self.line(level, f"{head} =")
            for ln in lines:
                self.line(level + 2, ln)

    def annotations(self, level: int, annotations) -> None:
        for name, value in annotations:
            self.blank()
            self.line(level, f"annotation {tq(name)} = {value}")

    def text(self) -> str:
        return "\n".join(self.lines) + "\n"


def column(name, dtype, *, source=None, expr=None, fmt=None, hidden=False, key=False, summarize="none",
           desc=None, sort_by=None, folder=None, category=None, inferred=False):
    return dict(name=name, dtype=dtype, source=source if source is not None or expr else name, expr=expr,
                fmt=fmt, hidden=hidden, key=key, summarize=summarize, desc=desc, sort_by=sort_by,
                folder=folder, category=category, inferred=inferred)


def measure(name, dax, fmt, folder=None, desc=None, hidden=False, fsd=None, annotations=()):
    return dict(name=name, dax=dax, fmt=fmt, folder=folder, desc=desc, hidden=hidden, fsd=fsd,
                annotations=annotations)


def m_source(blob: str, columns: int, steps: list[str], last: str) -> str:
    body = [
        "let",
        "    Source = AzureStorage.Blobs(StorageContainerUrl),",
        f'    #"Source CSV" = Csv.Document(Source{{[Name = "{blob}"]}}[Content], [Delimiter = ",", Columns = {columns}, Encoding = 65001, QuoteStyle = QuoteStyle.Csv]),',
        '    #"Promoted Headers" = Table.PromoteHeaders(#"Source CSV", [PromoteAllScalars = true]),',
    ]
    body += [f"    {s}," for s in steps[:-1]] + [f"    {steps[-1]}", "in", f"    {last}"]
    return "\n".join(body)


def renames(pairs: list[tuple[str, str]]) -> str:
    return "{" + ", ".join(f'{{"{a}", "{b}"}}' for a, b in pairs) + "}"


CURRENCY_HINT = ("PBI_FormatHint", '{"currencyCulture":"en-US"}')

ORDERS_M = m_source(BLOBS["orders"][0], 10, [
    '#"Removed Rows Without Id" = Table.SelectRows(#"Promoted Headers", each Text.Trim([order_id] ?? "") <> "")',
    '#"Cleaned Text" = Table.TransformColumns(#"Removed Rows Without Id", {{"order_id", Text.Trim, type text}, {"customer_id", fnCleanKey, type text}, {"store_id", fnCleanKey, type text}, {"product_id", fnCleanKey, type text}, {"status", each Text.Proper(Text.Trim(_)), type text}})',
    '#"Removed Duplicates" = Table.Distinct(#"Cleaned Text")',
    '#"Changed Type" = Table.TransformColumnTypes(#"Removed Duplicates", {{"order_date", type date}, {"ship_date", type date}, {"quantity", Int64.Type}, {"unit_price", type number}, {"discount", type number}})',
    '#"Default Discount" = Table.TransformColumns(#"Changed Type", {{"discount", each _ ?? 0, type number}})',
    '#"Added Line Amount" = Table.AddColumn(#"Default Discount", "line_amount", each [quantity] * [unit_price] * (1 - [discount]), type number)',
    '#"Renamed Columns" = Table.RenameColumns(#"Added Line Amount", ' + renames([
        ("order_id", "Order ID"), ("order_date", "Order Date"), ("ship_date", "Ship Date"),
        ("customer_id", "Customer ID"), ("store_id", "Store ID"), ("product_id", "Product ID"),
        ("quantity", "Quantity"), ("unit_price", "Unit Price"), ("discount", "Discount"),
        ("status", "Status"), ("line_amount", "Line Amount")]) + ")",
], '#"Renamed Columns"')

CUSTOMERS_M = m_source(BLOBS["customers"][0], 10, [
    '#"Cleaned Text" = Table.TransformColumns(#"Promoted Headers", {{"customer_id", fnCleanKey, type text}, {"email", each Text.Lower(Text.Trim(_)), type text}, {"segment", Text.Trim, type text}, {"loyalty_tier", each let t = Text.Trim(_ ?? "") in if t = "" then "Unassigned" else t, type text}, {"account_managers", each let names = fnParseNames(_ ?? "") in if names = "" then null else names, type nullable text}})',
    '#"Removed Duplicates" = Table.Distinct(#"Cleaned Text")',
    '#"Added Name" = Table.AddColumn(#"Removed Duplicates", "customer_name", each Text.Combine({Text.Trim([first_name]), Text.Trim([last_name])}, " "), type text)',
    '#"Added Domain" = Table.AddColumn(#"Added Name", "email_domain", each Text.AfterDelimiter([email], "@"), type text)',
    '#"Added Manager Count" = Table.AddColumn(#"Added Domain", "manager_count", each List.Count(List.RemoveNulls(List.Transform(Text.Split([account_managers] ?? "", ";"), each if Text.Trim(_) = "" then null else _))), Int64.Type)',
    '#"Changed Type" = Table.TransformColumnTypes(#"Added Manager Count", {{"join_date", type date}})',
    '#"Selected Columns" = Table.SelectColumns(#"Changed Type", {"customer_id", "customer_name", "email", "email_domain", "segment", "city", "state", "join_date", "loyalty_tier", "account_managers", "manager_count"})',
    '#"Renamed Columns" = Table.RenameColumns(#"Selected Columns", ' + renames([
        ("customer_id", "Customer ID"), ("customer_name", "Customer Name"), ("email", "Email"),
        ("email_domain", "Email Domain"), ("segment", "Segment"), ("city", "City"), ("state", "State"),
        ("join_date", "Join Date"), ("loyalty_tier", "Loyalty Tier"),
        ("account_managers", "Account Managers"), ("manager_count", "Account Manager Count")]) + ")",
], '#"Renamed Columns"')

PRODUCTS_M = m_source(BLOBS["products"][0], 7, [
    '#"Cleaned Text" = Table.TransformColumns(#"Promoted Headers", {{"product_id", fnCleanKey, type text}, {"product_name", Text.Trim, type text}, {"category", Text.Trim, type text}, {"subcategory", Text.Trim, type text}})',
    '#"Removed Duplicates" = Table.Distinct(#"Cleaned Text")',
    '#"Changed Type" = Table.TransformColumnTypes(#"Removed Duplicates", {{"list_price", Currency.Type}, {"unit_cost", Currency.Type}, {"launch_date", type date}})',
    '#"Renamed Columns" = Table.RenameColumns(#"Changed Type", ' + renames([
        ("product_id", "Product ID"), ("product_name", "Product Name"), ("category", "Category"),
        ("subcategory", "Subcategory"), ("list_price", "List Price"), ("unit_cost", "Unit Cost"),
        ("launch_date", "Launch Date")]) + ")",
], '#"Renamed Columns"')

STORES_M = m_source(BLOBS["stores"][0], 9, [
    '#"Cleaned Text" = Table.TransformColumns(#"Promoted Headers", {{"store_id", fnCleanKey, type text}, {"region", Text.Trim, type text}, {"manager_email", each Text.Lower(Text.Trim(_)), type text}})',
    '#"Removed Duplicates" = Table.Distinct(#"Cleaned Text")',
    '#"Blanks To Null" = Table.ReplaceValue(#"Removed Duplicates", "", null, Replacer.ReplaceValue, {"city", "state", "square_feet"})',
    '#"Changed Type" = Table.TransformColumnTypes(#"Blanks To Null", {{"open_date", type date}, {"square_feet", Int64.Type}})',
    '#"Added Manager" = Table.AddColumn(#"Changed Type", "store_manager", each Text.Proper(Text.Replace(Text.BeforeDelimiter([manager_email], "@"), ".", " ")), type text)',
    '#"Added Region Code" = Table.AddColumn(#"Added Manager", "region_code", each Text.Upper(Text.Start([region], 3)), type text)',
    '#"Renamed Columns" = Table.RenameColumns(#"Added Region Code", ' + renames([
        ("store_id", "Store ID"), ("store_name", "Store Name"), ("region", "Region"),
        ("channel", "Channel"), ("city", "City"), ("state", "State"), ("manager_email", "Manager Email"),
        ("open_date", "Open Date"), ("square_feet", "Square Feet"), ("store_manager", "Store Manager"),
        ("region_code", "Region Code")]) + ")",
], '#"Renamed Columns"')

TARGETS_M = m_source(BLOBS["targets"][0], 3, [
    '#"Removed Blank Rows" = Table.SelectRows(#"Promoted Headers", each Text.Trim([region] ?? "") <> "")',
    '#"Trimmed Region" = Table.TransformColumns(#"Removed Blank Rows", {{"region", Text.Trim, type text}})',
    '#"Added Month" = Table.AddColumn(#"Trimmed Region", "month_start", each Date.FromText([month] & "-01"), type date)',
    '#"Changed Type" = Table.TransformColumnTypes(#"Added Month", {{"target_amount", Currency.Type}})',
    '#"Selected Columns" = Table.SelectColumns(#"Changed Type", {"region", "month_start", "target_amount"})',
    '#"Renamed Columns" = Table.RenameColumns(#"Selected Columns", ' + renames([
        ("region", "Region"), ("month_start", "Month"), ("target_amount", "Target Amount")]) + ")",
], '#"Renamed Columns"')

RETURNS_M = m_source(BLOBS["returns"][0], 6, [
    '#"Cleaned Text" = Table.TransformColumns(#"Promoted Headers", {{"return_id", Text.Trim, type text}, {"order_id", Text.Trim, type text}, {"reason", each Text.Proper(Text.Trim(_)), type text}})',
    '#"Removed Duplicates" = Table.Distinct(#"Cleaned Text")',
    '#"Changed Type" = Table.TransformColumnTypes(#"Removed Duplicates", {{"return_date", type date}, {"quantity_returned", Int64.Type}, {"refund_amount", Currency.Type}})',
    '#"Renamed Columns" = Table.RenameColumns(#"Changed Type", ' + renames([
        ("return_id", "Return ID"), ("order_id", "Order ID"), ("return_date", "Return Date"),
        ("quantity_returned", "Quantity Returned"), ("reason", "Reason"),
        ("refund_amount", "Refund Amount")]) + ")",
], '#"Renamed Columns"')

SESSIONS_M = m_source(BLOBS["web_sessions"][0], 6, [
    '#"Cleaned Text" = Table.TransformColumns(#"Promoted Headers", {{"device", each Text.Proper(Text.Trim(_)), type text}})',
    '#"Changed Type" = Table.TransformColumnTypes(#"Cleaned Text", {{"session_date", type date}, {"pages_viewed", Int64.Type}, {"converted", type logical}})',
    '#"Added Campaign Flag" = Table.AddColumn(#"Changed Type", "is_campaign", each Text.Contains([traffic_source], "Campaign"), type logical)',
    '#"Renamed Columns" = Table.RenameColumns(#"Added Campaign Flag", ' + renames([
        ("session_id", "Session ID"), ("session_date", "Session Date"), ("device", "Device"),
        ("traffic_source", "Traffic Source"), ("pages_viewed", "Pages Viewed"),
        ("converted", "Converted"), ("is_campaign", "Is Campaign")]) + ")",
], '#"Renamed Columns"')

LAST_REFRESHED_M = "\n".join([
    "let",
    '    Source = #table(type table [#"Last Refreshed (UTC)" = datetime], {{DateTimeZone.RemoveZone(DateTimeZone.UtcNow())}})',
    "in",
    "    Source",
])

PARSE_NAMES_M = "\n".join([
    "(raw as text) as text =>",
    "let",
    "    Trimmed = Text.Trim(raw),",
    '    Inner = if Text.Start(Trimmed, 1) = "[" then Text.Range(Trimmed, 1, Text.Length(Trimmed) - 2) else Trimmed,',
    '    Names = List.Transform(Text.Split(Inner, ","), each Text.Trim(Text.Trim(_), {"\'", """"})),',
    '    NonEmpty = List.Select(Names, each _ <> ""),',
    '    Result = if List.IsEmpty(NonEmpty) then "" else Text.Combine(NonEmpty, "; ")',
    "in",
    "    Result",
])

DATE_DAX = "\n".join([
    "ADDCOLUMNS(",
    f"    CALENDAR(DATE({CAL_START.year}, {CAL_START.month}, {CAL_START.day}), DATE({CAL_END.year}, {CAL_END.month}, {CAL_END.day})),",
    '    "Year", YEAR([Date]),',
    '    "Month Number", MONTH([Date]),',
    '    "Month", FORMAT([Date], "mmm"),',
    '    "Year Month", FORMAT([Date], "yyyy-mm"),',
    '    "Quarter", "Q" & QUARTER([Date]),',
    '    "Month Start", DATE(YEAR([Date]), MONTH([Date]), 1)',
    ")",
])

NEVER_SOLD_DAX = "\n".join([
    "VAR SoldProducts =",
    '    SELECTCOLUMNS(SUMMARIZE(Orders, Orders[Product ID]), "Product ID", Orders[Product ID])',
    "VAR Catalog =",
    '    SELECTCOLUMNS(ALL(Products), "Product ID", Products[Product ID])',
    "VAR NeverSold = EXCEPT(Catalog, SoldProducts)",
    "RETURN",
    "    SELECTCOLUMNS(",
    "        FILTER(",
    "            ALL(Products),",
    "            Products[Product ID] IN NeverSold",
    "                && ISEMPTY(RELATEDTABLE(Orders))",
    "        ),",
    '        "Product ID", Products[Product ID],',
    '        "Product", Products[Product Name],',
    '        "Category", Products[Category],',
    '        "List Price", Products[List Price]',
    "    )",
])

SCALED_FSD = ('IF(ABS(SELECTEDMEASURE()) >= 1000000, "\\$#,0.00,,\\M", '
              'IF(ABS(SELECTEDMEASURE()) >= 1000, "\\$#,0.0,\\K", "\\$#,0.00"))')

TABLES = [
    dict(name="Orders", desc="One row per order line (synthetic). Cleaned in M: rows without an id and duplicates removed, keys trimmed and upper-cased, blank discounts set to 0.",
         partition=("m", ORDERS_M), hierarchies=[],
         columns=[
             column("Order ID", "string", key=True, desc="Order number (SO-…). Unique after Table.Distinct."),
             column("Order Date", "dateTime", fmt="yyyy-mm-dd"),
             column("Ship Date", "dateTime", fmt="yyyy-mm-dd", desc="Blank for cancelled and pending orders. Inactive relationship to 'Date' (USERELATIONSHIP)."),
             column("Customer ID", "string", hidden=True),
             column("Store ID", "string", hidden=True),
             column("Product ID", "string", hidden=True),
             column("Quantity", "int64", fmt="0", summarize="sum"),
             column("Unit Price", "double", fmt="\\$#,0.00"),
             column("Discount", "double", fmt="0%"),
             column("Status", "string", desc="Completed, Cancelled or Pending."),
             column("Line Amount", "double", fmt="\\$#,0.00", summarize="sum", desc="Quantity × unit price × (1 − discount), computed in M."),
             column("Line Cost", "double", expr="Orders[Quantity] * RELATED(Products[Unit Cost])", fmt="\\$#,0.00", summarize="sum", desc="Blank when the product is not in the catalog."),
             column("Order Size", "string", expr='SWITCH(TRUE(), Orders[Line Amount] >= 1000, "Large", Orders[Line Amount] >= 250, "Medium", "Small")'),
             column("Is Returned", "string", expr='IF(ISEMPTY(RELATEDTABLE(Returns)), "No", "Yes")', desc="Context transition through the one-to-one relationship to Returns."),
             column("Days To Ship", "int64", expr="IF(ISBLANK(Orders[Ship Date]), BLANK(), DATEDIFF(Orders[Order Date], Orders[Ship Date], DAY))", fmt="0"),
         ],
         measures=[
             measure("Total Sales", "SUM(Orders[Line Amount])", "\\$#,0.00", "Sales", "Sum of the order line amounts.", annotations=[CURRENCY_HINT]),
             measure("Total Cost", "SUM(Orders[Line Cost])", "\\$#,0.00", "Sales", annotations=[CURRENCY_HINT]),
             measure("Gross Margin", "[Total Sales] - [Total Cost]", "\\$#,0.00", "Sales", annotations=[CURRENCY_HINT]),
             measure("Margin %", "DIVIDE([Gross Margin], [Total Sales])", "0.0%", "Sales"),
             measure("Sales (Scaled)", "[Total Sales]", "\\$#,0.00", "Sales", "Total Sales with a dynamic format string: $1.23M, $45.6K or $789.00.", fsd=SCALED_FSD),
             measure("Completed Sales", 'CALCULATE([Total Sales], KEEPFILTERS(Orders[Status] = "Completed"))', "\\$#,0.00", "Sales"),
             measure("Shipped Sales", "CALCULATE([Total Sales], USERELATIONSHIP(Orders[Ship Date], 'Date'[Date]))", "\\$#,0.00", "Sales", "Sales by ship date (inactive relationship)."),
             measure("Sales All Stores", "CALCULATE([Total Sales], ALL(Stores))", "\\$#,0.00", "Sales", hidden=True),
             measure("Store Share", "DIVIDE([Total Sales], [Sales All Stores])", "0.0%", "Sales"),
             measure("Order Count", "COUNTROWS(Orders)", "#,0", "Orders"),
             measure("Units Sold", "SUM(Orders[Quantity])", "#,0", "Orders"),
             measure("Active Customers", "DISTINCTCOUNTNOBLANK(Orders[Customer ID])", "#,0", "Orders", "Distinct customer ids on orders, unknown ids included."),
             measure("Avg Order Value", "AVERAGEX(\n    VALUES(Orders[Order ID]),\n    CALCULATE(SUM(Orders[Line Amount]))\n)", "\\$#,0.00", "Orders"),
             measure("Revenue per Unit", "DIVIDE(\n    SUMX(Orders, Orders[Quantity] * Orders[Unit Price] * (1 - Orders[Discount])),\n    [Units Sold]\n)", "\\$#,0.00", "Orders"),
             measure("Last Order Date", "MAX(Orders[Order Date])", "yyyy-mm-dd", "Orders"),
             measure("Unknown Product Orders", "COUNTROWS(FILTER(Orders, ISBLANK(RELATED(Products[Product Name])))) + 0", "0", "Data Quality", "Orders whose product id is not in the catalog."),
             measure("Sales YTD", "TOTALYTD([Total Sales], 'Date'[Date])", "\\$#,0.00", "Time Intelligence"),
             measure("Sales PY", "CALCULATE([Total Sales], SAMEPERIODLASTYEAR('Date'[Date]))", "\\$#,0.00", "Time Intelligence"),
             measure("Sales YoY %", "DIVIDE([Total Sales] - [Sales PY], [Sales PY])", "+0.0%;-0.0%;0.0%", "Time Intelligence"),
             measure("Sales Prev Month", "CALCULATE([Total Sales], DATEADD('Date'[Date], -1, MONTH))", "\\$#,0.00", "Time Intelligence"),
         ]),
    dict(name="Customers", desc="Customers (synthetic). Account managers are parsed from Python-style lists by fnParseNames.",
         partition=("m", CUSTOMERS_M), hierarchies=[],
         columns=[
             column("Customer ID", "string", key=True),
             column("Customer Name", "string"),
             column("Email", "string"),
             column("Email Domain", "string"),
             column("Segment", "string"),
             column("City", "string", category="City"),
             column("State", "string", category="StateOrProvince"),
             column("Join Date", "dateTime", fmt="yyyy-mm-dd"),
             column("Loyalty Tier", "string", desc="Bronze, Silver, Gold; blanks become Unassigned."),
             column("Account Managers", "string", desc="Parsed names joined with \"; \"; blank when the list is empty."),
             column("Account Manager Count", "int64", fmt="0"),
             column("Lifetime Sales", "double", expr="CALCULATE(SUM(Orders[Line Amount]))", fmt="\\$#,0", summarize="sum", desc="Context transition: every order of the customer, all statuses."),
             column("Has Orders", "string", expr='IF(ISEMPTY(RELATEDTABLE(Orders)), "No", "Yes")'),
             column("Value Tier", "string", expr='SWITCH(\n    TRUE(),\n    Customers[Lifetime Sales] >= 3000, "High",\n    Customers[Lifetime Sales] >= 1000, "Medium",\n    Customers[Lifetime Sales] > 0, "Low",\n    "None"\n)'),
         ],
         measures=[
             measure("Customer Count", "COUNTROWS(Customers)", "#,0", None, "Customers in the filter context (cross-filtered by Orders, both directions)."),
         ]),
    dict(name="Products", desc="Product catalog: Category > Subcategory > Product.",
         partition=("m", PRODUCTS_M),
         hierarchies=[("Product Hierarchy", [("Category", "Category"), ("Subcategory", "Subcategory"), ("Product", "Product Name")])],
         columns=[
             column("Product ID", "string", key=True),
             column("Product Name", "string"),
             column("Category", "string"),
             column("Subcategory", "string"),
             column("List Price", "decimal", fmt="\\$#,0.00"),
             column("Unit Cost", "decimal", fmt="\\$#,0.00"),
             column("Launch Date", "dateTime", fmt="yyyy-mm-dd"),
             column("Units Sold (All Time)", "int64", expr="SUMX(RELATEDTABLE(Orders), Orders[Quantity])", fmt="#,0", summarize="sum"),
             column("Price Band", "string", expr='SWITCH(\n    TRUE(),\n    Products[List Price] < 50, "Under $50",\n    Products[List Price] < 150, "$50 to $149",\n    "$150 and up"\n)'),
         ],
         measures=[]),
    dict(name="Stores", desc="Stores and online channels. Manager Email drives the dynamic security role.",
         partition=("m", STORES_M),
         hierarchies=[("Geography", [("Region", "Region"), ("Store", "Store Name")])],
         columns=[
             column("Store ID", "string", key=True),
             column("Store Name", "string"),
             column("Region", "string"),
             column("Channel", "string", desc="Retail, Outlet or Online."),
             column("City", "string", category="City"),
             column("State", "string", category="StateOrProvince"),
             column("Manager Email", "string", hidden=True),
             column("Open Date", "dateTime", fmt="yyyy-mm-dd"),
             column("Square Feet", "int64", fmt="#,0"),
             column("Store Manager", "string"),
             column("Region Code", "string"),
             column("Store Label", "string", expr='Stores[Store Name] & " (" & Stores[Region] & ")"'),
         ],
         measures=[]),
    dict(name="Targets", desc="Monthly sales targets per region (many-to-many with Stores on Region).",
         partition=("m", TARGETS_M), hierarchies=[],
         columns=[
             column("Region", "string"),
             column("Month", "dateTime", fmt="yyyy-mm"),
             column("Target Amount", "decimal", fmt="\\$#,0", summarize="sum"),
         ],
         measures=[
             measure("Target", "SUM(Targets[Target Amount])", "\\$#,0", None, annotations=[CURRENCY_HINT]),
             measure("Target Attainment %", "DIVIDE([Total Sales], [Target])", "0.0%"),
             measure("Target Stretch", "[Target] * 1.2", "\\$#,0", None, "Gauge maximum.", hidden=True),
         ]),
    dict(name="Returns", desc="Returned orders: at most one return per order (one-to-one with Orders).",
         partition=("m", RETURNS_M), hierarchies=[],
         columns=[
             column("Return ID", "string"),
             column("Order ID", "string", key=True),
             column("Return Date", "dateTime", fmt="yyyy-mm-dd"),
             column("Quantity Returned", "int64", fmt="0", summarize="sum"),
             column("Reason", "string"),
             column("Refund Amount", "decimal", fmt="\\$#,0.00", summarize="sum"),
         ],
         measures=[
             measure("Returns Count", "COUNTROWS(Returns)", "#,0"),
             measure("Refunds", "SUM(Returns[Refund Amount])", "\\$#,0.00", annotations=[CURRENCY_HINT]),
             measure("Return Rate", "DIVIDE([Returns Count], [Order Count])", "0.0%"),
         ]),
    dict(name="Web Sessions", desc="Website and app sessions. No relationship to any other table: filters on it never reach the sales tables.",
         partition=("m", SESSIONS_M), hierarchies=[],
         columns=[
             column("Session ID", "string"),
             column("Session Date", "dateTime", fmt="yyyy-mm-dd"),
             column("Device", "string"),
             column("Traffic Source", "string"),
             column("Pages Viewed", "int64", fmt="0", summarize="sum"),
             column("Converted", "boolean"),
             column("Is Campaign", "boolean"),
         ],
         measures=[
             measure("Sessions", "COUNTROWS('Web Sessions')", "#,0"),
             measure("Converted Sessions", "CALCULATE([Sessions], 'Web Sessions'[Converted] = TRUE())", "#,0"),
             measure("Conversion Rate", "DIVIDE([Converted Sessions], [Sessions])", "0.0%"),
             measure("Avg Pages per Session", "AVERAGE('Web Sessions'[Pages Viewed])", "0.0"),
             measure("Sessions (All Devices)", "CALCULATE([Sessions], ALL('Web Sessions'[Device]))", "#,0", None, "Ignores filters on Device."),
         ]),
    dict(name="Date", desc="Calendar 2024-2025, marked as the date table.", category="Time",
         partition=("calculated", DATE_DAX),
         hierarchies=[("Calendar", [("Year", "Year"), ("Quarter", "Quarter"), ("Month", "Month"), ("Date", "Date")])],
         columns=[
             column("Date", "dateTime", source="[Date]", fmt="yyyy-mm-dd", key=True, inferred=True),
             column("Year", "int64", source="[Year]", fmt="0", inferred=True),
             column("Month Number", "int64", source="[Month Number]", fmt="0", hidden=True, inferred=True),
             column("Month", "string", source="[Month]", sort_by="Month Number", inferred=True),
             column("Year Month", "string", source="[Year Month]", inferred=True),
             column("Quarter", "string", source="[Quarter]", inferred=True),
             column("Month Start", "dateTime", source="[Month Start]", fmt="yyyy-mm-dd", inferred=True),
             column("Day Type", "string", expr='IF(WEEKDAY(\'Date\'[Date], 2) > 5, "Weekend", "Weekday")'),
         ],
         measures=[]),
    dict(name="Products Never Sold", desc="Calculated table: catalog products without any order.",
         partition=("calculated", NEVER_SOLD_DAX), hierarchies=[],
         columns=[
             column("Product ID", "string", source="[Product ID]", inferred=True),
             column("Product", "string", source="[Product]", inferred=True),
             column("Category", "string", source="[Category]", inferred=True),
             column("List Price", "decimal", source="[List Price]", fmt="\\$#,0.00", inferred=True),
         ],
         measures=[
             measure("Products Never Sold Count", "COUNTROWS('Products Never Sold') + 0", "0"),
         ]),
    dict(name="Last Refreshed", desc="Single row stamped with the refresh time (UTC). Not used by any expected value.",
         partition=("m", LAST_REFRESHED_M), hierarchies=[], hidden=True,
         columns=[column("Last Refreshed (UTC)", "dateTime", fmt="yyyy-mm-dd hh:nn")],
         measures=[]),
]

CALC_GROUP_ITEMS = [
    ("Current", "SELECTEDMEASURE()", None, "The measure as is."),
    ("YTD", "CALCULATE(SELECTEDMEASURE(), DATESYTD('Date'[Date]))", None, "Year to date."),
    ("PY", "CALCULATE(SELECTEDMEASURE(), SAMEPERIODLASTYEAR('Date'[Date]))", None, "Same period last year."),
    ("YoY %", "VAR CurrentValue = SELECTEDMEASURE()\nVAR PriorValue = CALCULATE(SELECTEDMEASURE(), SAMEPERIODLASTYEAR('Date'[Date]))\nRETURN\n    DIVIDE(CurrentValue - PriorValue, PriorValue)",
     '"+0.0%;-0.0%;0.0%"', "Change against the same period last year, always shown as a percent."),
]

# (from table, from column, to table, to column, options)
RELATIONSHIPS = [
    ("Orders", "Customer ID", "Customers", "Customer ID", {"crossFilteringBehavior": "bothDirections"}),
    ("Orders", "Product ID", "Products", "Product ID", {}),
    ("Orders", "Store ID", "Stores", "Store ID", {}),
    ("Orders", "Order Date", "Date", "Date", {}),
    ("Orders", "Ship Date", "Date", "Date", {"isActive": "false"}),
    ("Returns", "Order ID", "Orders", "Order ID", {"crossFilteringBehavior": "bothDirections", "fromCardinality": "one",
                                                  "toCardinality": "one", "securityFilteringBehavior": "bothDirections"}),
    ("Targets", "Region", "Stores", "Region", {"toCardinality": "many"}),
    ("Targets", "Month", "Date", "Date", {}),
]

ROLES = [
    ("Region West", "Sees the West region only: its stores, their orders, returns and targets.",
     'Stores[Region] = "West"'),
    ("Store Manager", "Dynamic security: a manager sees the stores whose Manager Email is the signed-in user.",
     "Stores[Manager Email] = USERPRINCIPALNAME()"),
]


def table_tmdl(t: dict) -> str:
    o = TmdlOut()
    name = t["name"]
    o.desc(0, t.get("desc"))
    o.line(0, f"table {tq(name)}")
    if t.get("hidden"):
        o.line(1, "isHidden")
    o.line(1, f"lineageTag: {guid('table', name)}")
    if t.get("category"):
        o.line(1, f"dataCategory: {t['category']}")
    for m in t["measures"]:
        MEASURE_HOME[m["name"]] = name
        o.blank()
        o.desc(1, m["desc"])
        o.expr(1, f"measure {tq(m['name'])}", m["dax"])
        o.line(2, f"formatString: {tmdl_text(m['fmt'])}")
        if m["folder"]:
            o.line(2, f"displayFolder: {tmdl_text(m['folder'])}")
        if m["hidden"]:
            o.line(2, "isHidden")
        o.line(2, f"lineageTag: {guid('measure', name, m['name'])}")
        if m["fsd"]:
            o.expr(2, "formatStringDefinition", m["fsd"])
        o.annotations(2, m["annotations"])
    for c in t["columns"]:
        o.blank()
        o.desc(1, c["desc"])
        head = f"column {tq(c['name'])}"
        if c["expr"]:
            o.expr(1, head, c["expr"])
        else:
            o.line(1, head)
        o.line(2, f"dataType: {c['dtype']}")
        if c["hidden"]:
            o.line(2, "isHidden")
        if c["key"]:
            o.line(2, "isKey")
        if c["inferred"]:
            o.line(2, "isNameInferred")
        if c["fmt"]:
            o.line(2, f"formatString: {tmdl_text(c['fmt'])}")
        if c["folder"]:
            o.line(2, f"displayFolder: {tmdl_text(c['folder'])}")
        o.line(2, f"lineageTag: {guid('column', name, c['name'])}")
        if c["category"]:
            o.line(2, f"dataCategory: {c['category']}")
        o.line(2, f"summarizeBy: {c['summarize']}")
        if not c["expr"]:
            o.line(2, f"sourceColumn: {tmdl_text(c['source'])}")
        if c["sort_by"]:
            o.line(2, f"sortByColumn: {tq(c['sort_by'])}")
        o.annotations(2, [("SummarizationSetBy", "Automatic")])
    for hname, levels in t["hierarchies"]:
        o.blank()
        o.line(1, f"hierarchy {tq(hname)}")
        o.line(2, f"lineageTag: {guid('hierarchy', name, hname)}")
        for lname, lcol in levels:
            o.blank()
            o.line(2, f"level {tq(lname)}")
            o.line(3, f"lineageTag: {guid('level', name, hname, lname)}")
            o.line(3, f"column: {tq(lcol)}")
    kind, source = t["partition"]
    o.blank()
    o.line(1, f"partition {tq(name)} = {kind}")
    o.line(2, "mode: import")
    if kind == "m":
        o.line(2, "queryGroup: Sources")
    o.expr(2, "source", source)
    if kind == "m":
        o.annotations(1, [("PBI_ResultType", "Table")])
    return o.text()


def calc_group_tmdl() -> str:
    o = TmdlOut()
    o.desc(0, "Calculation group applied to any measure: Current, YTD, PY and YoY %.")
    o.line(0, "table 'Time Calc'")
    o.line(1, f"lineageTag: {guid('table', 'Time Calc')}")
    o.blank()
    o.line(1, "calculationGroup")
    o.line(2, "precedence: 10")
    for i, (item, dax, fsd, desc) in enumerate(CALC_GROUP_ITEMS):
        o.blank()
        o.desc(2, desc)
        o.expr(2, f"calculationItem {tq(item)}", dax)
        o.line(3, f"ordinal: {i}")
        if fsd:
            o.expr(3, "formatStringDefinition", fsd)
    o.blank()
    o.line(1, "column 'Time Calculation'")
    o.line(2, "dataType: string")
    o.line(2, f"lineageTag: {guid('column', 'Time Calc', 'Time Calculation')}")
    o.line(2, "summarizeBy: none")
    o.line(2, "sourceColumn: Name")
    o.line(2, "sortByColumn: Ordinal")
    o.annotations(2, [("SummarizationSetBy", "Automatic")])
    o.blank()
    o.line(1, "column Ordinal")
    o.line(2, "dataType: int64")
    o.line(2, "isHidden")
    o.line(2, "formatString: 0")
    o.line(2, f"lineageTag: {guid('column', 'Time Calc', 'Ordinal')}")
    o.line(2, "summarizeBy: sum")
    o.line(2, "sourceColumn: Ordinal")
    o.annotations(2, [("SummarizationSetBy", "Automatic")])
    o.blank()
    o.line(1, "partition 'Time Calc' = calculationGroup")
    o.line(2, "mode: import")
    return o.text()


def write_model(out: Path) -> None:
    sm = out / f"{MODEL_NAME}.SemanticModel"
    d = sm / "definition"
    write_json(sm / "definition.pbism", {
        "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/semanticModel/definitionProperties/1.0.0/schema.json",
        "version": "4.2", "settings": {}})
    write_json(sm / ".platform", {
        "$schema": "https://developer.microsoft.com/json-schemas/fabric/gitIntegration/platformProperties/2.0.0/schema.json",
        "metadata": {"type": "SemanticModel", "displayName": MODEL_NAME},
        "config": {"version": "2.0", "logicalId": guid("item", "semantic-model")}})
    write_text(d / "database.tmdl", "database\n\tcompatibilityLevel: 1601\n")

    names = [t["name"] for t in TABLES]
    m_queries = [t["name"] for t in TABLES if t["partition"][0] == "m"]
    o = TmdlOut()
    o.desc(0, "Acme Outfitters: a synthetic outdoor-gear retailer (fictional data, DashCraft example).")
    o.line(0, "model Model")
    for ln in ["culture: en-US", "defaultPowerBIDataSourceVersion: powerBI_V3",
               "discourageImplicitMeasures", "sourceQueryCulture: en-US", "dataAccessOptions"]:
        o.line(1, ln)
    o.line(2, "legacyRedirects")
    o.line(2, "returnErrorValuesAsNull")
    for i, group in enumerate(["Parameters", "Functions", "Sources"]):
        o.blank()
        o.line(0, f"queryGroup {group}")
        o.annotations(1, [("PBI_QueryGroupOrder", str(i))])
    order = ["StorageContainerUrl", "fnCleanKey", "fnParseNames", *m_queries]
    o.annotations(0, [("__PBI_TimeIntelligenceEnabled", "0"),
                      ("PBI_QueryOrder", json.dumps(order, separators=(",", ":")))])
    o.blank()
    refs = list(names)
    refs.insert(refs.index("Date") + 1, "Time Calc")
    for n in refs:
        o.line(0, f"ref table {tq(n)}")
    o.blank()
    for r in ROLES:
        o.line(0, f"ref role {tq(r[0])}")
    write_text(d / "model.tmdl", o.text())

    o = TmdlOut()
    o.desc(0, "Fictional Azure Blob container with the sample CSVs. Never contacted: DashCraft resolves it\noffline through blob-map.json.")
    o.line(0, f'expression StorageContainerUrl = "{CONTAINER_URL}" meta [IsParameterQuery=true, Type="Text", IsParameterQueryRequired=true]')
    o.line(1, f"lineageTag: {guid('expression', 'StorageContainerUrl')}")
    o.line(1, "queryGroup: Parameters")
    o.annotations(1, [("PBI_ResultType", "Text")])
    o.blank()
    o.desc(0, "Trims and upper-cases a business key (customer, store and product ids).")
    o.line(0, "expression fnCleanKey = (x as text) as text => Text.Upper(Text.Trim(x))")
    o.line(1, f"lineageTag: {guid('expression', 'fnCleanKey')}")
    o.line(1, "queryGroup: Functions")
    o.annotations(1, [("PBI_ResultType", "Function")])
    o.blank()
    o.desc(0, "Turns a Python-style list of names, such as ['Ana Ruiz', \"O'Neil Park\"], into \"Ana Ruiz; O'Neil Park\".")
    o.expr(0, "expression fnParseNames", PARSE_NAMES_M)
    o.line(1, f"lineageTag: {guid('expression', 'fnParseNames')}")
    o.line(1, "queryGroup: Functions")
    o.annotations(1, [("PBI_ResultType", "Function")])
    write_text(d / "expressions.tmdl", o.text())

    o = TmdlOut()
    for i, (ft, fc, tt, tc, opts) in enumerate(RELATIONSHIPS):
        if i:
            o.blank()
        o.line(0, f"relationship {guid('relationship', ft, fc, tt, tc)}")
        for key in ("isActive", "crossFilteringBehavior", "fromCardinality", "toCardinality",
                    "securityFilteringBehavior"):
            if key in opts:
                o.line(1, f"{key}: {opts[key]}")
        o.line(1, f"fromColumn: {tq(ft)}.{tq(fc)}")
        o.line(1, f"toColumn: {tq(tt)}.{tq(tc)}")
    write_text(d / "relationships.tmdl", o.text())

    for t in TABLES:
        write_text(d / "tables" / f"{t['name']}.tmdl", table_tmdl(t))
    write_text(d / "tables" / "Time Calc.tmdl", calc_group_tmdl())

    for rname, rdesc, rfilter in ROLES:
        o = TmdlOut()
        o.desc(0, rdesc)
        o.line(0, f"role {tq(rname)}")
        o.line(1, "modelPermission: read")
        o.blank()
        o.line(1, f"tablePermission Stores = {rfilter}")
        write_text(d / "roles" / f"{rname}.tmdl", o.text())


def register_measures() -> None:
    for t in TABLES:
        for m in t["measures"]:
            MEASURE_HOME[m["name"]] = t["name"]
            assert m["name"] in MEASURES, m["name"]
            assert MEASURES[m["name"]][1] == m["fmt"], m["name"]
    assert set(MEASURE_HOME) == set(MEASURES), set(MEASURES) ^ set(MEASURE_HOME)


# ---------------------------------------------------------------------------
# Reports (PBIR)
# ---------------------------------------------------------------------------

GREEN, DARK, SAND, ORANGE, RED = "#2F6B4F", "#1F3A2E", "#D9C9A3", "#D9822B", "#B5452E"
NUM1 = Decimal(1)


def measure_expr(name: str) -> dict:
    return meas(name)


def fill_rule_gradient(name: str, low: str, high: str) -> dict:
    return {"solid": {"color": {"expr": {"FillRule": {"Input": measure_expr(name), "FillRule": {"linearGradient2": {
        "min": {"color": {"Literal": {"Value": f"'{low}'"}}},
        "max": {"color": {"Literal": {"Value": f"'{high}'"}}},
        "nullColoringStrategy": {"strategy": {"Literal": {"Value": "'asZero'"}}}}}}}}}}


def rules(name: str, cases: list[tuple[int, Decimal, str]], default: str) -> dict:
    return {"Conditional": {
        "Cases": [{"Condition": {"Comparison": {"ComparisonKind": kind, "Left": measure_expr(name),
                                                "Right": {"Literal": {"Value": lit(value)}}}},
                   "Value": {"Literal": {"Value": f"'{result}'"}}} for kind, value, result in cases],
        "DefaultValue": {"Literal": {"Value": f"'{default}'"}}}}


def color_rules(name: str, cases, default: str) -> dict:
    return {"solid": {"color": {"expr": rules(name, cases, default)}}}


def values_entry(query_ref_: str, props: dict, matching: int = 1) -> dict:
    return {"properties": props,
            "selector": {"data": [{"dataViewWildcard": {"matchingOption": matching}}], "metadata": query_ref_}}


def slicer_objects(mode: str, header: str, *, selection: F | None = None, single: bool = False,
                   inverted: bool = False, horizontal: bool = False) -> dict:
    data = {"mode": L(mode)}
    if inverted:
        data["isInvertedSelectionMode"] = L(True)
    o: dict = {
        "data": [{"properties": data}],
        "header": [{"properties": {"show": L(True), "text": L(header), "fontColor": color(DARK)}}],
        "items": [{"properties": {"fontColor": color("#252423")}}],
    }
    general: dict = {}
    if selection is not None:
        general["filter"] = {"filter": selection.definition()}
    if horizontal:
        general["orientation"] = L(NUM1)
    if general:
        o["general"] = [{"properties": general}]
    if single:
        o["selection"] = [{"properties": {"singleSelect": L(True), "strictSingleSelect": L(True)}}]
    elif mode in ("Basic", "Dropdown"):
        o["selection"] = [{"properties": {"selectAllCheckboxEnabled": L(True), "singleSelect": L(False)}}]
    return o


def card_visual_objects(tiles: int, size: int = 20, label_size: int = 10) -> dict:
    return {
        "layout": [{"properties": {"maxTiles": L(Decimal(tiles)), "alignment": L("center")}}],
        "value": [{"properties": {"fontSize": L(Decimal(size)), "fontColor": color(DARK),
                                  "labelDisplayUnits": L(NUM1)}}],
        "label": [{"properties": {"show": L(True), "position": L("belowValue"),
                                  "fontSize": L(Decimal(label_size))}}],
        "outline": [{"properties": {"show": L(False)}}],
        "accentBar": [{"properties": {"show": L(True), "color": color(GREEN)}}],
    }


def legacy_card_objects() -> dict:
    return {"labels": [{"properties": {"fontSize": L(Decimal(24)), "color": color(DARK),
                                       "labelDisplayUnits": L(NUM1)}}],
            "categoryLabels": [{"properties": {"show": L(True)}}]}


def chart_objects(*, labels: bool = True, legend: bool = False, point_fill: dict | None = None) -> dict:
    o: dict = {"labels": [{"properties": {"show": L(labels), "labelDisplayUnits": L(NUM1)}}],
               "legend": [{"properties": {"show": L(legend), "position": L("Top")}}]}
    if point_fill:
        o["dataPoint"] = [{"properties": {"fill": point_fill},
                           "selector": {"data": [{"dataViewWildcard": {"matchingOption": 1}}]}}]
    return o


def button(pg: Page, key: str, rect, action: dict, *, text: str | None = None, icon: str = "blank") -> Visual:
    objects = {
        "icon": [{"properties": {"shapeType": L(icon), "lineColor": color(DARK)}, "selector": {"id": "default"}}],
        "fill": [{"properties": {"show": L(True), "fillColor": color("#FFFFFF")}, "selector": {"id": "default"}}],
        "outline": [{"properties": {"show": L(True), "lineColor": color(GREEN)}, "selector": {"id": "default"}}],
        "shape": [{"properties": {"tileShape": L("rectangleRounded")}}],
    }
    if text:
        objects["text"] = [{"properties": {"show": L(True), "text": L(text), "fontColor": color(DARK)},
                            "selector": {"id": "default"}}]
    link = {"show": L(True), **{k: L(v) for k, v in action.items()}}
    container = vco(background=None, border=False)
    container["visualLink"] = [{"properties": link}]
    return pg.add(key, "actionButton", rect, objects=objects, container=container)


def banner(pg: Page, subtitle: str) -> None:
    pg.add("banner", "shape", (0, 0, pg.width, 64),
           objects={"shape": [{"properties": {"tileShape": L("rectangle")}}],
                    "fill": [{"properties": {"show": L(True), "fillColor": color(DARK), "transparency": L(Decimal(0))}}],
                    "outline": [{"properties": {"show": L(False)}}]},
           container=vco(background=None, border=False, radius=0))
    pg.add("title", "textbox", (24, 8, 680, 48),
           objects=textbox_paragraphs([{"textRuns": [
               run("Acme Outfitters", size="20pt", bold=True, color_hex="#FFFFFF"),
               run("  |  " + subtitle, size="14pt", color_hex=SAND)]}]),
           container=vco(background=None, border=False))


def overview_report() -> Report:
    rep = Report("overview", [F("Orders", "Status", "notin", ["Cancelled"])], "2.4.0")

    # Executive Summary -------------------------------------------------------------
    pg = rep.page("summary", "Executive Summary", 1280, 720, "FitToPage")
    products = rep.page("products", "Product Detail", 1280, 1080, "FitToWidth")
    banner(pg, "Executive summary")
    pg.add("navigator", "pageNavigator", (760, 14, 496, 36), container=vco(background=None, border=False))
    year = F("Date", "Year", "in", [2025])
    pg.add("year", "slicer", (24, 76, 360, 64), roles={"Values": [C("Date", "Year")]},
           objects=slicer_objects("Basic", "Year", selection=year, single=True, horizontal=True),
           selection=year, container=vco(None))
    pg.add("region", "slicer", (400, 76, 220, 64), roles={"Values": [C("Stores", "Region")]},
           objects=slicer_objects("Dropdown", "Region"), container=vco(None))
    pg.add("kpis", "cardVisual", (24, 152, 1000, 124),
           roles={"Data": [M("Total Sales"), M("Order Count"), M("Avg Order Value"), M("Margin %"),
                           M("Active Customers")]},
           objects=card_visual_objects(5), container=vco(None, shadow=True, radius=12))
    pg.add("scaled", "card", (1040, 152, 216, 124), roles={"Values": [M("Sales (Scaled)")]},
           objects=legacy_card_objects(), container=vco("Sales (dynamic format)", shadow=True, radius=12))
    pg.add("kpi", "kpi", (24, 292, 300, 192),
           roles={"Indicator": [M("Total Sales")], "TrendLine": [C("Date", "Year Month")], "Goal": [M("Target")]},
           objects={"indicator": [{"properties": {"indicatorDisplayUnits": L(NUM1)}}],
                    "trendline": [{"properties": {"show": L(True)}}],
                    "goals": [{"properties": {"showGoal": L(True), "showDistance": L(True)}}]},
           container=vco("Sales vs target (latest month)"))
    pg.add("trend", "lineChart", (340, 292, 600, 192),
           roles={"Category": [C("Date", "Month")], "Y": [M("Total Sales"), M("Sales PY")]},
           objects=chart_objects(labels=False, legend=True), container=vco("Sales by month, this year and last"))
    pg.add("by_category", "clusteredColumnChart", (956, 292, 300, 192),
           roles={"Category": [C("Products", "Category")], "Y": [M("Total Sales")]},
           objects=chart_objects(), container=vco("Sales by category"))
    pg.add("channel", "donutChart", (24, 496, 300, 208),
           roles={"Category": [C("Stores", "Channel")], "Y": [M("Total Sales")]},
           objects=chart_objects(legend=True), container=vco("Sales by channel"))
    pg.add("by_region", "clusteredBarChart", (340, 496, 600, 208),
           roles={"Category": [C("Stores", "Region")], "Y": [M("Total Sales"), M("Target")]},
           objects=chart_objects(legend=True), container=vco("Sales and target by region"))
    button(pg, "to_products", (956, 496, 300, 48), {"type": "PageNavigation", "navigationSection": products.id},
           text="Open product detail")
    pg.add("notes", "textbox", (956, 560, 300, 144), objects=textbox_paragraphs([
        {"textRuns": [run("About this report", size="12pt", bold=True, color_hex=DARK)]},
        {"textRuns": [run("Synthetic data, generated for DashCraft tests.")], "listType": "bullet"},
        {"textRuns": [run("Cancelled orders are filtered out at report level.")], "listType": "bullet"},
        {"textRuns": [run("Sample site: "), run("example.com", url="https://www.example.com/")]},
    ]), container=vco(None))
    pg.interact("by_category", "channel", "HighlightFilter")
    pg.interact("channel", "by_region", "DataFilter")
    pg.interact("by_region", "kpis", "NoFilter")

    # Product Detail ----------------------------------------------------------------
    pg = products
    banner(pg, "Product detail")
    button(pg, "back", (1208, 12, 48, 40), {"type": "Back"}, icon="back")
    cats = F("Products", "Category", "in", ["Camping", "Hiking"])
    price = F("Products", "List Price", "range", lo=Decimal(50), hi=Decimal(400))
    pg.add("category", "slicer", (24, 80, 220, 220), roles={"Values": [C("Products", "Category")]},
           objects=slicer_objects("Basic", "Category", selection=cats), selection=cats, container=vco(None))
    pg.add("price", "slicer", (24, 316, 220, 90), roles={"Values": [C("Products", "List Price")]},
           objects=slicer_objects("Between", "List price", selection=price), selection=price, container=vco(None))
    pg.add("kpis", "cardVisual", (260, 80, 760, 126),
           roles={"Data": [M("Total Sales"), M("Units Sold"), M("Margin %"), M("Revenue per Unit")]},
           objects=card_visual_objects(4, size=18), container=vco(None, shadow=True, radius=12))
    pg.add("never_sold", "cardVisual", (1036, 80, 220, 126), roles={"Data": [M("Products Never Sold Count")]},
           objects=card_visual_objects(1, label_size=9), container=vco(None, radius=12))
    pg.add("unknown", "card", (1036, 222, 220, 100), roles={"Values": [M("Unknown Product Orders")]},
           objects=legacy_card_objects(), container=vco("Orders with unknown products"))
    pg.add("matrix", "pivotTable", (260, 222, 760, 284),
           roles={"Rows": [H("Products", "Product Hierarchy", "Category"),
                           H("Products", "Product Hierarchy", "Subcategory")],
                  "Values": [M("Total Sales"), M("Units Sold"), M("Margin %")]},
           objects={
               "subTotals": [{"properties": {"rowSubtotals": L(True)}}],
               "rowHeaders": [{"properties": {"stepped": L(True)}}],
               "values": [
                   values_entry("Orders.Total Sales", {"backColor": fill_rule_gradient("Total Sales", "#F3EFE4", "#7FB394")}),
                   values_entry("Orders.Margin %", {"fontColor": color_rules(
                       "Margin %", [(3, Decimal("0.42"), RED), (2, Decimal("0.48"), GREEN)], "#252423")}),
               ]},
           container=vco("Sales by category and subcategory"))
    top10 = TopN("Products", "Product Name", 10, "Total Sales")
    pg.add("top10", "tableEx", (24, 522, 620, 300),
           roles={"Values": [C("Products", "Product Name"), M("Total Sales"), M("Units Sold"), M("Margin %")]},
           filters=[top10], sort=[(M("Total Sales"), "Descending")],
           objects={"values": [
               values_entry("Orders.Total Sales", {"dataBars": {
                   "positiveColor": color("#7FB394"), "negativeColor": color(RED), "axisColor": color("#252423"),
                   "reverseDirection": L(False), "hideText": L(False)}}),
               values_entry("Orders.Margin %", {"icon": {
                   "kind": "Icon", "layout": L("Before"), "verticalAlignment": L("Middle"),
                   "value": {"expr": {"Conditional": {"Cases": [
                       {"Condition": {"Comparison": {"ComparisonKind": 2, "Left": measure_expr("Margin %"),
                                                     "Right": {"Literal": {"Value": "0.48D"}}}},
                        "Value": {"Literal": {"Value": "'circle_green'"}}},
                       {"Condition": {"Comparison": {"ComparisonKind": 2, "Left": measure_expr("Margin %"),
                                                     "Right": {"Literal": {"Value": "0.42D"}}}},
                        "Value": {"Literal": {"Value": "'circle_yellow'"}}}],
                       "DefaultValue": {"Literal": {"Value": "'circle_red'"}}}}}}})]},
           container=vco("Top 10 products by sales"))
    pg.add("treemap", "treemap", (660, 522, 596, 300),
           roles={"Group": [C("Products", "Subcategory")], "Values": [M("Total Sales")]},
           objects=chart_objects(), container=vco("Sales by subcategory"))
    pg.add("scatter", "scatterChart", (24, 838, 620, 220),
           roles={"Category": [C("Products", "Product Name")], "Series": [C("Products", "Category")],
                  "X": [M("Units Sold")], "Y": [M("Total Sales")]},
           objects=chart_objects(labels=False, legend=True), container=vco("Units against sales per product"))
    pg.add("units_cf", "clusteredBarChart", (660, 838, 596, 220),
           roles={"Category": [C("Products", "Subcategory")], "Y": [M("Units Sold")]},
           objects=chart_objects(point_fill=color_rules("Margin %", [(3, Decimal("0.44"), RED)], GREEN)),
           container=vco("Units by subcategory (red: margin below 44%)"))

    # Time Intelligence --------------------------------------------------------------
    pg = rep.page("time", "Time Intelligence", 1280, 720, "FitToPage")
    banner(pg, "Time intelligence")
    month = F("Date", "Year Month", "in", ["2025-06"])
    pg.add("month", "slicer", (24, 76, 220, 64), roles={"Values": [C("Date", "Year Month")]},
           objects=slicer_objects("Dropdown", "Month", selection=month, single=True), selection=month,
           container=vco(None))
    relative = F("Date", "Date", "relative", years=10)
    pg.add("relative", "slicer", (260, 76, 320, 64), roles={"Values": [C("Date", "Date")]},
           objects=slicer_objects("Relative", "Order date (relative)", selection=relative), selection=relative,
           container=vco(None))
    for i, (key, item) in enumerate([("current", "Current"), ("ytd", "YTD"), ("py", "PY"), ("yoy", "YoY %")]):
        x = 24 + i * 316
        pg.add(key, "cardVisual", (x, 152, 300 if i < 3 else 284, 100), roles={"Data": [M("Total Sales")]},
               filters=[F("Time Calc", "Time Calculation", "in", [item])],
               objects=card_visual_objects(1), container=vco(f"Total Sales - {item}", radius=12))
    pg.add("measures", "multiRowCard", (24, 268, 400, 200),
           roles={"Values": [M("Sales YTD"), M("Sales PY"), M("Sales YoY %"), M("Sales Prev Month"),
                             M("Shipped Sales")]},
           container=vco("Same values as measures"))
    pg.add("calc_matrix", "pivotTable", (440, 268, 816, 200),
           roles={"Rows": [C("Date", "Year"), C("Date", "Month")],
                  "Columns": [C("Time Calc", "Time Calculation")], "Values": [M("Total Sales")]},
           objects={"subTotals": [{"properties": {"rowSubtotals": L(True)}}]},
           container=vco("Total Sales by time calculation (all months)"))
    pg.add("line", "lineChart", (24, 484, 616, 220),
           roles={"Category": [C("Date", "Year Month")], "Y": [M("Total Sales"), M("Sales PY")]},
           filters=[F("Date", "Date", "relative", years=10)],
           objects=chart_objects(labels=False, legend=True), container=vco("Sales by month (last 10 years)"))
    pg.add("area", "areaChart", (656, 484, 600, 220),
           roles={"Category": [C("Date", "Year Month")], "Y": [M("Shipped Sales")]},
           objects=chart_objects(labels=False), container=vco("Shipped sales by ship month"))
    for target in ("calc_matrix", "line", "area"):
        pg.interact("month", target, "NoFilter")
    for v in pg.visuals:
        if v.key not in ("relative", "banner", "title"):
            pg.interact("relative", v.key, "NoFilter")
    return rep


def stores_report() -> Report:
    rep = Report("stores", [F("Stores", "Channel", "in", ["Retail", "Outlet"])], "2.11.0")
    pg = rep.page("performance", "Store Performance", 1280, 720, "FitToPage")
    detail = rep.page("detail", "Store Detail", 1280, 900, "FitToWidth",
                      filters=[F("Stores", "Region", "in", ["East"])], visibility="HiddenInViewMode")
    tip = rep.page("tooltip", "Store Tooltip", 320, 240, "ActualSize", visibility="HiddenInViewMode",
                   page_type="Tooltip")
    banner(pg, "Store performance")
    west_bm, all_bm = hex20("stores", "bookmark", "west"), hex20("stores", "bookmark", "all")
    button(pg, "bookmark_west", (1000, 14, 120, 36), {"type": "Bookmark", "bookmark": west_bm},
           text="West region", icon="bookmark")
    button(pg, "bookmark_all", (1136, 14, 120, 36), {"type": "Bookmark", "bookmark": all_bm},
           text="All regions", icon="bookmark")
    region = F("Stores", "Region", "in", ["West"])
    pg.add("region", "slicer", (24, 76, 240, 64), roles={"Values": [C("Stores", "Region")]},
           objects=slicer_objects("Dropdown", "Region", selection=region, single=True), selection=region,
           container=vco(None))
    dates = F("Date", "Date", "range", lo=date(2025, 1, 1), hi=date(2026, 1, 1), hi_inclusive=False)
    pg.add("dates", "slicer", (24, 148, 240, 64), roles={"Values": [C("Date", "Date")]},
           objects=slicer_objects("Between", "Order date", selection=dates), selection=dates, container=vco(None))
    pg.add("kpis", "cardVisual", (280, 76, 976, 136),
           roles={"Data": [M("Total Sales"), M("Target"), M("Target Attainment %"), M("Return Rate"),
                           M("Gross Margin")]},
           objects=card_visual_objects(5, size=18), container=vco(None, shadow=True, radius=12))
    pg.add("stores", "tableEx", (24, 228, 560, 236),
           roles={"Values": [C("Stores", "Store Name"), M("Total Sales"), M("Target"), M("Target Attainment %")]},
           sort=[(M("Total Sales"), "Descending")],
           objects={"values": [
               values_entry("Orders.Total Sales", {"backColor": fill_rule_gradient("Total Sales", "#FFFFFF", "#9CC5AE")}),
               values_entry("Targets.Target Attainment %", {"fontColor": color_rules(
                   "Target Attainment %", [(3, Decimal("0.3"), RED)], GREEN)}),
           ]},
           container=vco("Stores (target = the store's region target)"))
    pg.add("stacked", "stackedColumnChart", (600, 228, 330, 236),
           roles={"Category": [C("Stores", "Store Name")], "Y": [M("Total Sales")],
                  "Series": [C("Products", "Category")]},
           objects=chart_objects(labels=False, legend=True),
           container=vco("Sales by store and category", tooltip_page=tip.id))
    pg.add("gauge", "gauge", (946, 228, 310, 236),
           roles={"Y": [M("Total Sales")], "TargetValue": [M("Target")], "MaxValue": [M("Target Stretch")]},
           container=vco("Sales against target"))
    pg.add("combo", "lineClusteredColumnComboChart", (24, 480, 300, 224),
           roles={"Category": [C("Date", "Year Month")], "Y": [M("Total Sales")], "Y2": [M("Target")]},
           sort=[(C("Date", "Year Month"), "Ascending")],
           objects=chart_objects(labels=False, legend=True), container=vco("Monthly sales and target"))
    pg.add("waterfall", "waterfallChart", (340, 480, 300, 224),
           roles={"Category": [C("Date", "Quarter")], "Y": [M("Gross Margin")]},
           objects=chart_objects(), container=vco("Gross margin by quarter"))
    pg.add("segments", "hundredPercentStackedBarChart", (656, 480, 300, 224),
           roles={"Category": [C("Stores", "Store Name")], "Y": [M("Total Sales")],
                  "Series": [C("Customers", "Segment")]},
           objects=chart_objects(labels=False, legend=True), container=vco("Customer segment mix"))
    pg.add("map", "map", (972, 480, 284, 224),
           roles={"Category": [C("Stores", "City")], "Size": [M("Total Sales")]},
           container=vco("Store map (not rendered offline)"))
    pg.interact("stacked", "combo", "NoFilter")
    pg.interact("stacked", "stores", "DataFilter")
    pg.interact("segments", "combo", "HighlightFilter")
    pg.interact("waterfall", "gauge", "NoFilter")
    rep.bookmarks = [
        {"id": west_bm, "name": "West region", "page": pg.id, "slicer": pg.v("region").id, "selection": region},
        {"id": all_bm, "name": "All regions", "page": pg.id, "slicer": pg.v("region").id, "selection": None},
    ]

    pg = detail
    banner(pg, "Store detail (hidden page, East region)")
    pg.add("stores", "multiRowCard", (24, 80, 1232, 280),
           roles={"Values": [C("Stores", "Store Name"), C("Stores", "Store Manager"), C("Stores", "City"),
                             M("Total Sales")]},
           objects={"card": [{"properties": {"barShow": L(True), "barColor": color(GREEN)}}]},
           container=vco("East stores"))
    pg.add("returns", "tableEx", (24, 376, 600, 300),
           roles={"Values": [C("Returns", "Reason"), M("Returns Count"), M("Refunds")]},
           sort=[(M("Returns Count"), "Descending")], container=vco("Returns by reason"))
    pg.add("reasons", "clusteredColumnChart", (640, 376, 616, 300),
           roles={"Category": [C("Returns", "Reason")], "Y": [M("Returns Count")]},
           objects=chart_objects(), container=vco("Returns by reason"))

    pg = tip
    pg.add("tip", "cardVisual", (8, 8, 304, 224), roles={"Data": [M("Total Sales"), M("Target Attainment %")]},
           objects=card_visual_objects(1), container=vco(None, radius=12))
    return rep


def online_report() -> Report:
    rep = Report("online", [F("Stores", "Channel", "in", ["Online"]),
                            F("Web Sessions", "Device", "in", ["Mobile"])], "2.12.0")
    pg = rep.page("online", "Online Overview", 1280, 720, "FitToPage")
    customers = rep.page("customers", "Customer Detail", 1280, 960, "FitToWidth",
                         filters=[F("Customers", "Segment", "in", ["Corporate"])])
    banner(pg, "Online (report filters: online channel, mobile sessions)")
    button(pg, "reset", (1136, 14, 120, 36), {"type": "ClearAllSlicers"}, text="Reset", icon="reset")
    tier = F("Customers", "Loyalty Tier", "notin", ["Unassigned"])
    pg.add("tier", "slicer", (24, 76, 200, 300), roles={"Values": [C("Customers", "Loyalty Tier")]},
           objects=slicer_objects("Basic", "Loyalty tier", selection=tier, inverted=True), selection=tier,
           container=vco(None))
    pg.add("kpis", "cardVisual", (240, 76, 1016, 124),
           roles={"Data": [M("Total Sales"), M("Order Count"), M("Active Customers"),
                           [M("Customer Count"), "Customers Reached"], M("Avg Order Value")]},
           objects=card_visual_objects(5), container=vco(None, shadow=True, radius=12))
    pg.add("sessions", "card", (240, 216, 240, 124), roles={"Values": [M("Sessions")]},
           objects=legacy_card_objects(), container=vco("Mobile sessions"))
    pg.add("web", "cardVisual", (496, 216, 500, 124),
           roles={"Data": [M("Conversion Rate"), M("Avg Pages per Session"), M("Sessions (All Devices)")]},
           objects=card_visual_objects(3, label_size=8), container=vco(None, radius=12))
    pg.add("note", "textbox", (1012, 216, 244, 124), objects=textbox_paragraphs([
        {"textRuns": [run("Report filters", bold=True)]},
        {"textRuns": [run("Channel = Online")], "listType": "bullet"},
        {"textRuns": [run("Device = Mobile (sessions only)")], "listType": "bullet"},
    ]), container=vco(None))
    pg.add("funnel", "funnel", (240, 356, 330, 348),
           roles={"Category": [C("Web Sessions", "Traffic Source")], "Y": [M("Sessions")]},
           objects=chart_objects(), container=vco("Sessions by traffic source"))
    pg.add("devices", "pieChart", (586, 356, 330, 168),
           roles={"Category": [C("Web Sessions", "Device")], "Y": [M("Sessions")]},
           objects=chart_objects(legend=True), container=vco("Sessions by device"))
    pg.add("segments", "clusteredBarChart", (932, 356, 324, 168),
           roles={"Category": [C("Customers", "Segment")], "Y": [M("Total Sales")]},
           objects=chart_objects(), container=vco("Online sales by segment"))
    pg.add("tiers", "hundredPercentStackedColumnChart", (586, 540, 670, 164),
           roles={"Category": [C("Date", "Quarter")], "Y": [M("Total Sales")],
                  "Series": [C("Customers", "Loyalty Tier")]},
           sort=[(C("Date", "Quarter"), "Ascending")],
           objects=chart_objects(labels=False, legend=True), container=vco("Loyalty mix by quarter"))
    pg.add("arrow", "basicShape", (24, 392, 200, 24),
           objects={"general": [{"properties": {"shapeType": L("arrow")}}],
                    "fill": [{"properties": {"show": L(True), "fillColor": color(SAND)}}],
                    "line": [{"properties": {"lineColor": color(GREEN), "weight": L(NUM1)}}]},
           container=vco(background=None, border=False))
    pg.add("help", "textbox", (24, 432, 200, 272), objects=textbox_paragraphs([
        {"textRuns": [run("How to read this page", bold=True, color_hex=DARK)]},
        {"textRuns": [run("The tier slicer excludes Unassigned (inverted selection).")], "listType": "number"},
        {"textRuns": [run("Session visuals ignore the sales filters: Web Sessions has no relationships.")],
         "listType": "number"},
        {"textRuns": [run("Docs: "), run("example.org", url="https://www.example.org/")]},
    ]), container=vco(None))

    pg = customers
    banner(pg, "Corporate customers (page filter)")
    button(pg, "back", (1208, 12, 48, 40), {"type": "Back"}, icon="back")
    pg.add("count", "cardVisual", (24, 80, 480, 124), roles={"Data": [M("Customer Count"), M("Total Sales")]},
           objects=card_visual_objects(2), container=vco(None, radius=12))
    pg.add("table", "tableEx", (24, 220, 760, 400),
           roles={"Values": [C("Customers", "Customer Name"), C("Customers", "Account Managers"),
                             C("Customers", "Account Manager Count"), M("Total Sales")]},
           sort=[(M("Total Sales"), "Descending")], container=vco("Corporate customers buying online"))
    pg.add("matrix", "pivotTable", (800, 220, 456, 200),
           roles={"Rows": [C("Customers", "Segment")], "Columns": [C("Customers", "Loyalty Tier")],
                  "Values": [M("Total Sales")]},
           container=vco("Sales by segment and tier"))
    pg.add("states", "stackedBarChart", (800, 436, 456, 184),
           roles={"Category": [C("Customers", "State")], "Y": [M("Total Sales")],
                  "Series": [C("Customers", "Value Tier")]},
           objects=chart_objects(labels=False, legend=True), container=vco("Sales by state and value tier"))
    return rep


# ---------------------------------------------------------------------------
# Expected values
# ---------------------------------------------------------------------------


def card_entries(e: Engine, pg: Page, key: str, names: list[str]) -> list[dict]:
    vis = pg.v(key)
    ctx = pg.ctx(key)
    out = []
    for n in names:
        entry = {"visual": vis.id, "visualKey": key, "visualType": vis.type, "measure": n}
        entry.update(e.value(n, ctx))
        out.append(entry)
    return out


def ctx_plus(ctx: Ctx, table: str, column: str, value) -> Ctx:
    return ctx.add(table, column, lambda r, v=value: r.get(column) == v)


def page_expectations(e: Engine, rep: Report, pg: Page) -> dict:
    cards: list[dict] = []
    visuals: list[dict] = []
    key = (rep.key, pg.key)
    if key == ("overview", "summary"):
        cards += card_entries(e, pg, "kpis", ["Total Sales", "Order Count", "Avg Order Value", "Margin %",
                                              "Active Customers"])
        cards += card_entries(e, pg, "scaled", ["Sales (Scaled)"])
        ctx = pg.ctx("kpi")
        months = sorted({DATE_ROWS[o["Order Date"]]["Year Month"] for o in e.orders(ctx)})
        last = months[-1]
        mctx = ctx_plus(ctx, "Date", "Year Month", last)
        visuals.append({"visual": pg.v("kpi").id, "visualKey": "kpi", "visualType": "kpi",
                        "description": f"Indicator and goal at the last trend point ({last})",
                        "trendPoint": last, "indicator": e.value("Total Sales", mctx),
                        "goal": e.value("Target", mctx)})
    elif key == ("overview", "products"):
        cards += card_entries(e, pg, "kpis", ["Total Sales", "Units Sold", "Margin %", "Revenue per Unit"])
        cards += card_entries(e, pg, "never_sold", ["Products Never Sold Count"])
        cards += card_entries(e, pg, "unknown", ["Unknown Product Orders"])
        ctx = pg.ctx("matrix")
        names = ["Total Sales", "Units Sold", "Margin %"]
        cats = sorted({o["_product"]["Category"] for o in e.orders(ctx) if o["_product"]})
        visuals.append({
            "visual": pg.v("matrix").id, "visualKey": "matrix", "visualType": "pivotTable",
            "description": "Grand total and category subtotals",
            "total": {n: e.value(n, ctx) for n in names},
            "rows": [{"Category": c, **{n: e.value(n, ctx_plus(ctx, "Products", "Category", c)) for n in names}}
                     for c in cats]})
        ctx = pg.ctx("top10")
        sales: dict[str, Decimal] = {}
        for o in e.orders(ctx):
            if o["_product"]:
                name = o["_product"]["Product Name"]
                sales[name] = sales.get(name, Decimal(0)) + o["Line Amount"]
        ranked = sorted(sales.items(), key=lambda kv: (-kv[1], kv[0]))
        assert ranked[9][1] != ranked[10][1], "Top N tie"
        top = [n for n, _ in ranked[:10]]
        tctx = ctx.add("Products", "Product Name", lambda r, s=frozenset(top): r.get("Product Name") in s)
        visuals.append({
            "visual": pg.v("top10").id, "visualKey": "top10", "visualType": "tableEx",
            "description": "Top 10 products by Total Sales (Top N visual filter), sorted descending",
            "rows": [{"Product Name": n, **{m: e.value(m, ctx_plus(ctx, "Products", "Product Name", n))
                                            for m in names}} for n in top],
            "total": {m: e.value(m, tctx) for m in names}})
    elif key == ("overview", "time"):
        for k in ("current", "ytd", "py", "yoy"):
            entry = card_entries(e, pg, k, ["Total Sales"])[0]
            entry["calculationItem"] = pg.ctx(k).item
            cards.append(entry)
        cards += card_entries(e, pg, "measures", ["Sales YTD", "Sales PY", "Sales YoY %", "Sales Prev Month",
                                                  "Shipped Sales"])
        ctx = pg.ctx("calc_matrix")
        items = [i[0] for i in CALC_GROUP_ITEMS]
        visuals.append({
            "visual": pg.v("calc_matrix").id, "visualKey": "calc_matrix", "visualType": "pivotTable",
            "description": "Total Sales per calculation item: year rows and the grand total (month slicer does not filter it)",
            "rows": [{"Year": y, **{i: e.value("Total Sales", ctx_plus(ctx, "Date", "Year", y).replace(item=i))
                                    for i in items}} for y in (2024, 2025)],
            "total": {i: e.value("Total Sales", ctx.replace(item=i)) for i in items}})
    elif key == ("stores", "performance"):
        cards += card_entries(e, pg, "kpis", ["Total Sales", "Target", "Target Attainment %", "Return Rate",
                                              "Gross Margin"])
        ctx = pg.ctx("stores")
        names = ["Total Sales", "Target", "Target Attainment %"]
        stores = [s for s in e.t["Stores"] if ctx.ok("Stores", s)]
        rows = [{"Store Name": s["Store Name"],
                 **{n: e.value(n, ctx_plus(ctx, "Stores", "Store Name", s["Store Name"])) for n in names}}
                for s in stores]
        rows.sort(key=lambda r: -(r["Total Sales"]["value"] or 0))
        visuals.append({"visual": pg.v("stores").id, "visualKey": "stores", "visualType": "tableEx",
                        "description": "Per store: Target is the store's region target (many-to-many); the total is not the sum of the rows",
                        "rows": rows, "total": {n: e.value(n, ctx) for n in names}})
        ctx = pg.ctx("gauge")
        visuals.append({"visual": pg.v("gauge").id, "visualKey": "gauge", "visualType": "gauge",
                        "value": e.value("Total Sales", ctx), "target": e.value("Target", ctx),
                        "max": e.value("Target Stretch", ctx)})
    elif key == ("stores", "detail"):
        ctx = pg.ctx("stores")
        stores = [s for s in e.t["Stores"] if ctx.ok("Stores", s)]
        visuals.append({"visual": pg.v("stores").id, "visualKey": "stores", "visualType": "multiRowCard",
                        "rows": [{"Store Name": s["Store Name"], "Store Manager": s["Store Manager"],
                                  "City": s["City"],
                                  "Total Sales": e.value("Total Sales", ctx_plus(ctx, "Stores", "Store ID", s["Store ID"]))}
                                 for s in stores]})
        ctx = pg.ctx("returns")
        reasons = sorted({r["Reason"] for r in e.returns(ctx)})
        rows = [{"Reason": r, **{n: e.value(n, ctx.add("Returns", "Reason", lambda x, r=r: x["Reason"] == r))
                                 for n in ("Returns Count", "Refunds")}} for r in reasons]
        visuals.append({"visual": pg.v("returns").id, "visualKey": "returns", "visualType": "tableEx",
                        "rows": rows,
                        "total": {n: e.value(n, ctx) for n in ("Returns Count", "Refunds")}})
    elif key == ("stores", "tooltip"):
        cards += card_entries(e, pg, "tip", ["Total Sales", "Target Attainment %"])
    elif key == ("online", "online"):
        cards += card_entries(e, pg, "kpis", ["Total Sales", "Order Count", "Active Customers", "Customer Count",
                                              "Avg Order Value"])
        cards += card_entries(e, pg, "sessions", ["Sessions"])
        cards += card_entries(e, pg, "web", ["Conversion Rate", "Avg Pages per Session", "Sessions (All Devices)"])
        ctx = pg.ctx("devices")
        devices = sorted({s["Device"] for s in e.sessions(ctx)})
        visuals.append({"visual": pg.v("devices").id, "visualKey": "devices", "visualType": "pieChart",
                        "description": "Only the Mobile slice: the report filter on the unrelated table applies to session visuals",
                        "categories": [{"Device": d, "Sessions": e.value("Sessions", ctx_plus(ctx, "Web Sessions", "Device", d))}
                                       for d in devices]})
    elif key == ("online", "customers"):
        cards += card_entries(e, pg, "count", ["Customer Count", "Total Sales"])
        ctx = pg.ctx("table")
        groups: dict[tuple, Decimal] = {}
        for o in e.orders(ctx):
            c = o["_customer"]
            k = (c["Customer Name"], c["Account Managers"], c["Account Manager Count"])
            groups[k] = groups.get(k, Decimal(0)) + o["Line Amount"]
        visuals.append({"visual": pg.v("table").id, "visualKey": "table", "visualType": "tableEx",
                        "rowCount": len(groups), "total": {"Total Sales": e.value("Total Sales", ctx)}})
        ctx = pg.ctx("matrix")
        tiers = sorted({o["_customer"]["Loyalty Tier"] for o in e.orders(ctx)})
        visuals.append({"visual": pg.v("matrix").id, "visualKey": "matrix", "visualType": "pivotTable",
                        "columns": [{"Loyalty Tier": t, "Total Sales": e.value(
                            "Total Sales", ctx_plus(ctx, "Customers", "Loyalty Tier", t))} for t in tiers],
                        "total": {"Total Sales": e.value("Total Sales", ctx)}})
    filters = [f.describe() for f in pg.filters]
    slicers = []
    for v in pg.visuals:
        if v.selection is None:
            continue
        entry = {"visual": v.id, "visualKey": v.key, "selection": v.selection.describe()}
        blocked = [t for s_, t, k in pg.interactions if s_ == v.key and k == "NoFilter"]
        if blocked:
            entry["noFilterTargets"] = blocked
        slicers.append(entry)
    return {"page": pg.display, "pageId": pg.id, "displayOption": pg.option,
            "visibility": pg.visibility or "AlwaysVisible", "type": pg.page_type or "Default",
            "pageFilters": filters, "slicers": slicers, "cards": cards, "visuals": visuals}


def all_measures(e: Engine, ctx: Ctx) -> dict:
    return {name: e.value(name, ctx) for name in MEASURES}


def rls_expectations(e: Engine, tables: dict, targets: list[dict]) -> list[dict]:
    cases = [
        (["Region West"], None, lambda s: s["Region"] == "West"),
        (["Store Manager"], DYNAMIC_RLS_USER, lambda s: s["Manager Email"] == DYNAMIC_RLS_USER),
        (["Store Manager"], f"nobody@{EMAIL_DOMAIN}", lambda s: s["Manager Email"] == f"nobody@{EMAIL_DOMAIN}"),
        (["Region West", "Store Manager"], DYNAMIC_RLS_USER,
         lambda s: s["Region"] == "West" or s["Manager Email"] == DYNAMIC_RLS_USER),
    ]
    out = []
    for roles, user, pred in cases:
        ctx = Ctx().add("Stores", "*security*", pred)
        rows = {
            "Orders": len(e.orders(ctx)), "Customers": len(tables["Customers"]),
            "Products": len(tables["Products"]), "Stores": len([s for s in tables["Stores"] if pred(s)]),
            "Targets": len(e.targets(ctx)), "Returns": len(e.returns(ctx)),
            "Web Sessions": len(tables["Web Sessions"]), "Date": len(CALENDAR),
            "Products Never Sold": len(e.never_sold), "Last Refreshed": 1,
            "Time Calc": len(CALC_GROUP_ITEMS),
        }
        out.append({"roles": roles, "userName": user, "visibleRows": rows,
                    "measures": {n: e.value(n, ctx) for n in
                                 ("Total Sales", "Order Count", "Target", "Returns Count", "Sessions")}})
    return out


def build_expected(e: Engine, tables: dict, targets: list[dict], reports: list[Report], raw: dict,
                   targets_raw: list) -> dict:
    row_counts = {
        "Orders": len(tables["Orders"]), "Customers": len(tables["Customers"]),
        "Products": len(tables["Products"]), "Stores": len(tables["Stores"]), "Targets": len(targets),
        "Returns": len(tables["Returns"]), "Web Sessions": len(tables["Web Sessions"]),
        "Date": len(CALENDAR), "Products Never Sold": len(e.never_sold), "Time Calc": len(CALC_GROUP_ITEMS),
        "Last Refreshed": 1,
    }
    raw_counts = {Path(BLOBS[k][1]).name: len(v) for k, v in raw.items() if k in BLOBS}
    raw_counts["targets.csv"] = len(targets_raw)
    ci = {i[0]: None for i in CALC_GROUP_ITEMS}
    time_page = next(r for r in reports if r.key == "overview").page_by("time")
    month_ctx = time_page.ctx("current").replace(item=None)
    for item in ci:
        ci[item] = e.value("Total Sales", month_ctx.replace(item=item))
    online = next(r for r in reports if r.key == "online")
    channel_only = Ctx()
    channel_only = online.filters[0].apply(channel_only)
    both = online.filters[1].apply(channel_only)
    unmatched_customers = sorted({o["Customer ID"] for o in tables["Orders"] if o["_customer"] is None})
    return {
        "schemaVersion": 1,
        "generator": "scripts/examples/gen_acme.py",
        "seed": SEED,
        "dataAsOf": DATA_AS_OF.isoformat(),
        "notes": [
            "Values are computed by the generator in plain Python over the generated rows, not by DashCraft.",
            "'formatted' applies the measure's format string (display units None), as Power BI shows it.",
            "Card contexts: report filters + page filters + saved slicer selections (honouring visualInteractions) + visual filters.",
            "Relative date filters ('last 10 years') keep every row: the data ends 2025-12-31.",
            "A Not In filter (inverted slicer) keeps the blank member of unmatched keys, as Power BI does.",
            "'knownEngineGap' marks an expectation DashCraft does not meet yet (see EXPECTED.md).",
        ],
        "pbip": f"{MODEL_NAME}.pbip",
        "blobMap": "blob-map.json",
        "rowCounts": row_counts,
        "rawCsvRows": raw_counts,
        "dataQuality": {
            "unmatchedCustomerIds": unmatched_customers,
            "ordersWithUnmatchedCustomer": len([o for o in tables["Orders"] if o["_customer"] is None]),
            "ordersWithUnknownProduct": len([o for o in tables["Orders"] if o["_product"] is None]),
            "productsNeverSold": e.never_sold,
        },
        "calculatedTables": {"Products Never Sold": {"rows": len(e.never_sold), "productIds": e.never_sold},
                             "Date": {"rows": len(CALENDAR), "from": CAL_START.isoformat(), "to": CAL_END.isoformat()}},
        "measures": {
            "unfiltered": all_measures(e, Ctx()),
            "byReport": {r.name: {"reportFilters": [f.describe() for f in r.filters],
                                  "values": all_measures(e, _report_ctx(r))}
                         for r in reports},
        },
        "calculationGroup": {
            "table": "Time Calc", "column": "Time Calculation", "measure": "Total Sales",
            "context": "Acme Outfitters - Overview, page Time Intelligence: report filter + Year Month = 2025-06",
            "month": "2025-06", "items": ci,
        },
        "rls": rls_expectations(e, tables, targets),
        "unrelatedReportFilter": {
            "report": online.name, "filter": online.filters[1].describe(),
            "Sessions": {"withReportFilters": e.value("Sessions", both),
                         "withoutDeviceFilter": e.value("Sessions", channel_only)},
            "Sessions (All Devices)": e.value("Sessions (All Devices)", both),
            "Total Sales": {"withReportFilters": e.value("Total Sales", both),
                            "withoutDeviceFilter": e.value("Total Sales", channel_only)},
        },
        "reports": [{"report": r.name, "folder": f"{r.name}.Report",
                     "reportFilters": [f.describe() for f in r.filters],
                     "pages": [page_expectations(e, r, p) for p in r.pages]} for r in reports],
    }


def _report_ctx(r: Report) -> Ctx:
    c = Ctx()
    for f in r.filters:
        c = f.apply(c)
    return c


# ---------------------------------------------------------------------------
# Writers: reports, theme, project files, expected values
# ---------------------------------------------------------------------------

THEME = {
    "name": "Acme Outfitters",
    "dataColors": ["#2F6B4F", "#D9822B", "#4A90A4", "#8E4A6B", "#8FA857", "#5B6770", "#C9A227",
                   "#B5452E", "#6C8EBF", "#A07E5E"],
    "foreground": "#252423",
    "foregroundNeutralSecondary": "#5B6770",
    "background": "#FFFFFF",
    "backgroundLight": "#F3EFE4",
    "tableAccent": "#2F6B4F",
    "good": "#2F6B4F",
    "neutral": "#C9A227",
    "bad": "#B5452E",
    "hyperlink": "#2C6E8F",
    "textClasses": {
        "callout": {"fontSize": 24, "fontFace": "Segoe UI Semibold", "color": "#1F3A2E"},
        "title": {"fontSize": 12, "fontFace": "Segoe UI Semibold", "color": "#1F3A2E"},
        "header": {"fontSize": 12, "fontFace": "Segoe UI Semibold", "color": "#1F3A2E"},
        "label": {"fontSize": 10, "fontFace": "Segoe UI", "color": "#252423"},
    },
    "visualStyles": {
        "*": {"*": {
            "border": [{"show": True, "color": {"solid": {"color": "#D9D4C7"}}, "radius": 8}],
            "background": [{"show": True, "color": {"solid": {"color": "#FFFFFF"}}, "transparency": 0}],
        }},
        "page": {"*": {"background": [{"color": {"solid": {"color": "#F7F5EF"}}, "transparency": 0}]}},
    },
}
THEME_FILE = "AcmeOutfitters.json"


def write_report(out: Path, rep: Report) -> None:
    root = out / f"{rep.name}.Report"
    d = root / "definition"
    write_json(root / "definition.pbir", {
        "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definitionProperties/2.0.0/schema.json",
        "version": "4.0",
        "datasetReference": {"byPath": {"path": f"../{MODEL_NAME}.SemanticModel"}}})
    write_json(root / ".platform", {
        "$schema": "https://developer.microsoft.com/json-schemas/fabric/gitIntegration/platformProperties/2.0.0/schema.json",
        "metadata": {"type": "Report", "displayName": rep.name},
        "config": {"version": "2.0", "logicalId": guid("item", "report", rep.key)}})
    write_json(root / "StaticResources" / "RegisteredResources" / THEME_FILE, THEME)
    write_json(d / "version.json", {"$schema": f"{SCHEMA}/versionMetadata/1.0.0/schema.json", "version": "2.0.0"})
    report: dict = {
        "$schema": f"{SCHEMA}/report/3.3.0/schema.json",
        "themeCollection": {"customTheme": {
            "name": THEME_FILE, "reportVersionAtImport": {"visual": "2.4.0", "report": "3.3.0", "page": "2.1.0"},
            "type": "RegisteredResources"}},
    }
    if rep.filters:
        report["filterConfig"] = {"filters": [f.card(hex20(rep.key, "report-filter", str(i)))
                                              for i, f in enumerate(rep.filters)]}
    report["resourcePackages"] = [{"name": "RegisteredResources", "type": "RegisteredResources",
                                   "items": [{"name": THEME_FILE, "path": THEME_FILE, "type": "CustomTheme"}]}]
    report["settings"] = {"useStylableVisualContainerHeader": True, "exportDataMode": "AllowSummarized",
                          "defaultDrillFilterOtherVisuals": True, "allowChangeFilterTypes": True,
                          "useEnhancedTooltips": True, "useDefaultAggregateDisplayName": True}
    write_json(d / "report.json", report)
    write_json(d / "pages" / "pages.json", {
        "$schema": f"{SCHEMA}/pagesMetadata/1.1.0/schema.json",
        "pageOrder": [p.id for p in rep.pages],
        "activePageName": rep.pages[0].id})
    for p in rep.pages:
        write_json(d / "pages" / p.id / "page.json", p.json())
        for v in p.visuals:
            write_json(d / "pages" / p.id / "visuals" / v.id / "visual.json", v.json)
    if rep.bookmarks:
        write_json(d / "bookmarks" / "bookmarks.json", {
            "$schema": f"{SCHEMA}/bookmarksMetadata/1.0.0/schema.json",
            "items": [{"name": b["id"]} for b in rep.bookmarks]})
        for b in rep.bookmarks:
            if b["selection"] is not None:
                objects = {"merge": {"general": [{"properties": {"filter": {"filter": b["selection"].definition()}}}]}}
            else:
                objects = {"remove": [{"object": "general", "property": "filter", "selector": {}}]}
            container = {"singleVisual": {"visualType": "slicer", "objects": objects}}
            write_json(d / "bookmarks" / f"{b['id']}.bookmark.json", {
                "$schema": f"{SCHEMA}/bookmark/1.4.0/schema.json",
                "displayName": b["name"],
                "name": b["id"],
                "options": {"targetVisualNames": [b["slicer"]], "applyOnlyToTargetVisuals": True},
                "explorationState": {"version": "1.3", "activeSection": b["page"], "sections": {
                    b["page"]: {"visualContainers": {b["slicer"]: container}}}}})


def write_project(out: Path) -> None:
    write_text(out / ".gitignore", "**/.pbi/localSettings.json\n**/.pbi/cache.abf\n")
    write_json(out / f"{MODEL_NAME}.pbip", {
        "$schema": "https://developer.microsoft.com/json-schemas/fabric/pbip/pbipProperties/1.0.0/schema.json",
        "version": "1.0",
        "artifacts": [{"report": {"path": f"{REPORT_NAMES['overview']}.Report"}}],
        "settings": {"enableAutoRecovery": True}})
    write_json(out / "blob-map.json", {
        "$comment": "Offline AzureStorage.Blobs mapping for the fictional Acme Outfitters container. Paths are relative to this file. The URL is a placeholder and is never contacted.",
        "containers": {CONTAINER_URL: {blob: path for blob, path in BLOBS.values()}}})


SOURCE_OF = {
    "Orders": "M: `orders.csv` (3 rows without id, 16 duplicates removed)",
    "Customers": "M: `customers.csv` (3 duplicates removed)",
    "Products": "M: `products.csv` (2 duplicates removed)",
    "Stores": "M: `stores.csv` (1 duplicate removed)",
    "Targets": "M: `targets.csv` (1 blank row removed)",
    "Returns": "M: `returns.csv` (1 duplicate removed)",
    "Web Sessions": "M: `web_sessions.csv`",
    "Date": "DAX calculated table (CALENDAR + ADDCOLUMNS)",
    "Products Never Sold": "DAX calculated table",
    "Time Calc": "calculation group",
    "Last Refreshed": "M: #table with DateTimeZone.UtcNow()",
}


def md_table(headers: list[str], rows: list[list]) -> str:
    out = ["| " + " | ".join(headers) + " |", "|" + "|".join("---" for _ in headers) + "|"]
    out += ["| " + " | ".join(str(c) for c in r) + " |" for r in rows]
    return "\n".join(out)


def write_expected_md(out: Path, exp: dict) -> None:
    lines = [
        "# Acme Outfitters: expected values",
        "",
        "Generated by `scripts/examples/gen_acme.py` (do not edit by hand). The values are computed in",
        "plain Python over the generated rows, independently of DashCraft; `expected.json` has the same",
        "data plus raw values. Data is synthetic and ends on " + exp["dataAsOf"] + ".",
        "",
        "## Row counts after the M steps",
        "",
        md_table(["Table", "Rows", "Source"],
                 [[t, n, SOURCE_OF.get(t, "")] for t, n in exp["rowCounts"].items()]),
        "",
        "Raw CSV data rows (header excluded): "
        + ", ".join(f"`{k}` {v}" for k, v in exp["rawCsvRows"].items()) + ".",
        "",
        "## KPI cards per report page",
        "",
    ]
    for r in exp["reports"]:
        lines += [f"### {r['report']}", "", "Report filters: " + ("; ".join(r["reportFilters"]) or "none"), ""]
        for p in r["pages"]:
            if not p["cards"] and not p["visuals"]:
                continue
            ctx = p["pageFilters"] + [f"slicer {s['selection']}" + (
                f" (does not filter: {', '.join(s['noFilterTargets'])})" if s.get("noFilterTargets") else "")
                for s in p["slicers"]]
            lines += [f"**{p['page']}** ({p['displayOption']}, {p['visibility']}, {p['type']}). "
                      + ("Context: " + "; ".join(ctx) if ctx else "No page filters or slicer selections."), ""]
            if p["cards"]:
                rows = []
                for c in p["cards"]:
                    label = c["measure"] + (f" [{c['calculationItem']}]" if c.get("calculationItem") else "")
                    gap = f" ⚠ {c['knownEngineGap']}" if c.get("knownEngineGap") else ""
                    rows.append([c["visualKey"], c["visualType"], label, c["formatted"] + gap])
                lines += [md_table(["Visual", "Type", "Measure", "Expected"], rows), ""]
            for v in p["visuals"]:
                gap = f" ⚠ known engine gap: {v['knownEngineGap']}" if v.get("knownEngineGap") else ""
                lines.append(f"- `{v['visualKey']}` ({v['visualType']}): " + v.get("description", "") + gap)
                if "total" in v:
                    lines.append("  - total: " + ", ".join(f"{k} {x['formatted']}" for k, x in v["total"].items()))
                if "indicator" in v:
                    lines.append(f"  - indicator {v['indicator']['formatted']}, goal {v['goal']['formatted']}")
                if "value" in v:
                    lines.append(f"  - value {v['value']['formatted']}, target {v['target']['formatted']}, "
                                 f"max {v['max']['formatted']}")
                if "rowCount" in v:
                    lines.append(f"  - rows: {v['rowCount']}")
                for row in v.get("rows", [])[:12]:
                    lines.append("  - " + ", ".join(
                        f"{k} {x['formatted'] if isinstance(x, dict) else x}" for k, x in row.items()))
                for col_ in v.get("columns", []) + v.get("categories", []):
                    lines.append("  - " + ", ".join(
                        f"{k} {x['formatted'] if isinstance(x, dict) else x}" for k, x in col_.items()))
            lines.append("")
    cg = exp["calculationGroup"]
    lines += ["## Calculation group", "", f"`'{cg['table']}'[{cg['column']}]` applied to [{cg['measure']}]; "
              + cg["context"] + ".", "",
              md_table(["Item", "Expected"], [[k, v["formatted"]] for k, v in cg["items"].items()]), ""]
    lines += ["## Row-level security", "",
              "Roles filter `Stores`; the filter reaches Orders, Returns (one-to-one, security both directions)",
              "and Targets (many-to-many on Region). Customers are not filtered: the Orders-Customers relationship",
              "cross-filters both ways but applies security in one direction only.", ""]
    tables = list(exp["rls"][0]["visibleRows"])
    lines.append(md_table(["Roles", "User"] + tables + ["Total Sales", "Target"],
                          [[", ".join(c["roles"]), c["userName"] or "", *[c["visibleRows"][t] for t in tables],
                            c["measures"]["Total Sales"]["formatted"], c["measures"]["Target"]["formatted"]]
                           for c in exp["rls"]]))
    u = exp["unrelatedReportFilter"]
    lines += ["", "## Report filter on an unrelated table", "",
              f"{u['report']}: `{u['filter']}`. Web Sessions has no relationship, so the filter changes only the",
              "session measures:", "",
              md_table(["Measure", "With the report filters", "Without the Device filter"], [
                  ["Sessions", u["Sessions"]["withReportFilters"]["formatted"], u["Sessions"]["withoutDeviceFilter"]["formatted"]],
                  ["Total Sales", u["Total Sales"]["withReportFilters"]["formatted"], u["Total Sales"]["withoutDeviceFilter"]["formatted"]],
              ]), "",
              f"Sessions (All Devices) under the report filters: {u['Sessions (All Devices)']['formatted']}.", "",
              "## Measures", "", "Unfiltered and under each report's report-level filters (`dashcraft measures --report`).", ""]
    names = list(exp["measures"]["unfiltered"])
    by = exp["measures"]["byReport"]
    lines.append(md_table(["Measure", "Unfiltered"] + [n.replace("Acme Outfitters - ", "") for n in by],
                          [[n, exp["measures"]["unfiltered"][n]["formatted"]]
                           + [by[r]["values"][n]["formatted"] for r in by] for n in names]))
    gaps = exp.get("knownEngineGaps", [])
    if gaps:
        lines += ["", "## Known engine gaps", ""] + [f"- {g}" for g in gaps]
    lines += ["", "## Data quality built into the CSVs", "",
              f"- Orders with an unknown customer id ({', '.join(exp['dataQuality']['unmatchedCustomerIds'])}): "
              f"{exp['dataQuality']['ordersWithUnmatchedCustomer']}",
              f"- Orders with an unknown product id: {exp['dataQuality']['ordersWithUnknownProduct']}",
              f"- Products never sold: {', '.join(exp['dataQuality']['productsNeverSold'])}",
              "- Duplicates (exact and after trimming/casing), rows without an order id, blank discounts,",
              "  blank loyalty tiers, Python-style manager lists, trailing spaces and mixed casing in keys.", ""]
    write_text(out / "EXPECTED.md", "\n".join(lines))


# Expectations DashCraft does not meet yet: (report key, page key, visual key, measure or None) -> reason.
KNOWN_ENGINE_GAPS: dict[tuple, str] = {}


def mark_gaps(exp: dict) -> None:
    notes = []
    for r in exp["reports"]:
        rkey = next(k for k, v in REPORT_NAMES.items() if v == r["report"])
        for p in r["pages"]:
            for c in p["cards"]:
                reason = KNOWN_ENGINE_GAPS.get((rkey, p["page"], c["visualKey"], c["measure"]))
                if reason:
                    c["knownEngineGap"] = reason
                    notes.append(f"{r['report']} / {p['page']} / {c['visualKey']} [{c['measure']}]: {reason}")
            for v in p["visuals"]:
                reason = KNOWN_ENGINE_GAPS.get((rkey, p["page"], v["visualKey"], None))
                if reason:
                    v["knownEngineGap"] = reason
                    notes.append(f"{r['report']} / {p['page']} / {v['visualKey']}: {reason}")
    for key, reason in KNOWN_ENGINE_GAPS.items():
        if key[0] == "*":
            notes.append(f"{key[1]}: {reason}")
    exp["knownEngineGaps"] = notes


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = ap.parse_args()
    out: Path = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    for name in GENERATED:
        p = out / name
        if p.is_dir():
            shutil.rmtree(p)
        elif p.exists():
            p.unlink()

    rng = random.Random(SEED)
    raw = generate_sources(rng)
    tables = clean_tables(raw)
    targets_raw = make_targets(rng, tables["Orders"])
    raw["targets"] = targets_raw
    targets = clean_targets(targets_raw)
    for key in HEADERS:
        write_csv(out / BLOBS[key][1], HEADERS[key], raw[key])

    register_measures()
    engine = Engine(tables, targets, raw["never_sold"])
    reports = [overview_report(), stores_report(), online_report()]

    write_model(out)
    for rep in reports:
        write_report(out, rep)
    write_project(out)

    exp = build_expected(engine, tables, targets, reports, {k: v for k, v in raw.items() if k in HEADERS},
                         targets_raw)
    mark_gaps(exp)
    if BOUNDARY_ISSUES:
        raise SystemExit("values on a rounding boundary (change the data): " + "; ".join(BOUNDARY_ISSUES))
    write_json(out / "expected.json", exp)
    write_expected_md(out, exp)
    print(f"wrote {out.relative_to(ROOT) if out.is_relative_to(ROOT) else out}")


if __name__ == "__main__":
    main()
