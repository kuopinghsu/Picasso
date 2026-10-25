#!/usr/bin/env python3
"""Builds build/world.bin, the country and state/province shapes behind the
Travel Map page, from Natural Earth's public-domain 1:50m GeoJSON.

    python3 -m venv /tmp/ne-venv && /tmp/ne-venv/bin/pip install mapbox-earcut
    /tmp/ne-venv/bin/python scripts/world-map.py [DOWNLOAD_DIR]

The GeoJSON files are downloaded into DOWNLOAD_DIR (default /tmp/ne) when
they are missing. The output is zlib-compressed; src/world.rs reads it.

Layout (little-endian): b"PWM1", u16 feature count, then per feature
    u8-length id, u16 parent index (0xFFFF for countries), u8 continent,
    10 u16-length names (Picasso's language order), u16 x/y label point,
    u32 vertex count + u16 x/y pairs, u16 ring count + u32 start/length,
    u32 index count + i32 triangle indices.
x maps longitude -180..180 and y latitude -90..90 onto 0..65535. Each vertex
is stored as the wrapping difference from the previous one and each index as
the difference from the previous index, which compresses far better.
"""

import json
import math
import os
import struct
import sys
import urllib.request
import zlib

import mapbox_earcut
import numpy as np

BASE = "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson/"
COUNTRIES = "ne_50m_admin_0_countries.geojson"
REGIONS = "ne_50m_admin_1_states_provinces.geojson"
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "build", "world.bin")

# Picasso's language order: en, zh-Hant, zh-Hans, ja, ko, de, es, fr, it, pt-BR.
NAME_FIELDS = ["EN", "ZHT", "ZH", "JA", "KO", "DE", "ES", "FR", "IT", "PT"]
CONTINENTS = {"Africa": 0, "Americas": 1, "Asia": 2, "Europe": 3, "Oceania": 4, "Antarctica": 4}
# Natural Earth calls Taiwan "Taiwan" in English but uses the formal name in
# Chinese, Japanese and Korean; use the matching short name everywhere.
NAME_OVERRIDES = {"TW": {"ZHT": "臺灣", "ZH": "台湾", "JA": "台湾", "KO": "대만"}}
MAX_EDGE = 1.0  # degrees between neighbouring outline points
MAX_TRIANGLE_EDGE = 2.0  # degrees; keeps triangles small enough to bend over the globe


def load(directory, name):
    path = os.path.join(directory, name)
    if not os.path.exists(path):
        os.makedirs(directory, exist_ok=True)
        print("downloading", name)
        urllib.request.urlretrieve(BASE + name, path)
    with open(path, encoding="utf-8") as file:
        return json.load(file)["features"]


def quantize(lon, lat):
    x = round((max(-180.0, min(180.0, lon)) + 180.0) / 360.0 * 65535.0)
    y = round((max(-90.0, min(90.0, lat)) + 90.0) / 180.0 * 65535.0)
    return x, y


def degrees(point):
    return point[0] * 360.0 / 65535.0 - 180.0, point[1] * 180.0 / 65535.0 - 90.0


def polygons(geometry):
    if geometry["type"] == "Polygon":
        return [geometry["coordinates"]]
    return geometry["coordinates"]


def ring_points(ring):
    """Quantized ring without the closing point or repeats, with long edges split."""
    points = []
    coords = ring[:-1] if ring[0] == ring[-1] else ring
    for index, (lon, lat) in enumerate(coords):
        next_lon, next_lat = coords[(index + 1) % len(coords)]
        across = abs(next_lon - lon) * math.cos(math.radians((lat + next_lat) / 2))
        steps = max(1, math.ceil(max(across, abs(next_lat - lat)) / MAX_EDGE))
        for step in range(steps):
            t = step / steps
            point = quantize(lon + (next_lon - lon) * t, lat + (next_lat - lat) * t)
            if not points or points[-1] != point:
                points.append(point)
    while len(points) > 1 and points[0] == points[-1]:
        points.pop()
    return points if len(points) >= 3 else None


def too_long(a, b):
    """Whether the edge spans more than MAX_TRIANGLE_EDGE of arc on the globe;
    a degree of longitude shrinks toward the poles."""
    (ax, ay), (bx, by) = degrees(a), degrees(b)
    across = abs(ax - bx) * math.cos(math.radians((ay + by) / 2))
    return max(across, abs(ay - by)) > MAX_TRIANGLE_EDGE


def subdivide(vertices, triangles):
    """Splits triangle edges longer than MAX_TRIANGLE_EDGE. A shared edge is
    split the same way on both sides, so no cracks open between triangles."""
    midpoints = {}

    def midpoint(i, j):
        key = (min(i, j), max(i, j))
        if key not in midpoints:
            (ax, ay), (bx, by) = vertices[i], vertices[j]
            vertices.append(((ax + bx) // 2, (ay + by) // 2))
            midpoints[key] = len(vertices) - 1
        return midpoints[key]

    out = []
    stack = list(triangles)
    while stack:
        a, b, c = stack.pop()
        long_ab = too_long(vertices[a], vertices[b])
        long_bc = too_long(vertices[b], vertices[c])
        long_ca = too_long(vertices[c], vertices[a])
        count = long_ab + long_bc + long_ca
        if count == 0:
            out.append((a, b, c))
        elif count == 3:
            ab, bc, ca = midpoint(a, b), midpoint(b, c), midpoint(c, a)
            stack += [(a, ab, ca), (ab, b, bc), (ca, bc, c), (ab, bc, ca)]
        elif count == 1:
            # Rotate so the long edge is a-b.
            if long_bc:
                a, b, c = b, c, a
            elif long_ca:
                a, b, c = c, a, b
            ab = midpoint(a, b)
            stack += [(a, ab, c), (ab, b, c)]
        else:
            # Rotate so the short edge is c-a; a-b and b-c are long.
            if not long_ab:
                a, b, c = b, c, a
            elif not long_bc:
                a, b, c = c, a, b
            ab, bc = midpoint(a, b), midpoint(b, c)
            stack += [(ab, b, bc), (a, ab, bc), (a, bc, c)]
    return out


def shapes(geometry):
    vertices, rings, triangles = [], [], []
    for polygon in polygons(geometry):
        outer = ring_points(polygon[0])
        if not outer:
            continue
        parts = [outer] + [points for points in (ring_points(ring) for ring in polygon[1:]) if points]
        base = len(vertices)
        ends, flat = [], []
        for points in parts:
            rings.append((len(vertices), len(points)))
            vertices.extend(points)
            flat.extend(points)
            ends.append(len(flat))
        array = np.array(flat, dtype=np.float64).reshape(-1, 2)
        found = mapbox_earcut.triangulate_float64(array, np.array(ends, dtype=np.uint32))
        triangles += [tuple(base + int(i) for i in found[k : k + 3]) for k in range(0, len(found), 3)]
    triangles = subdivide(vertices, triangles)
    return vertices, rings, triangles


def names(properties, upper, override):
    english = properties["NAME_EN" if upper else "name_en"] or properties["NAME" if upper else "name"]
    out = []
    for field in NAME_FIELDS:
        key = "NAME_" + field if upper else "name_" + field.lower()
        out.append(override.get(field) or properties.get(key) or english)
    return out


def two_letters(value):
    return len(value) == 2 and value.isalpha() and value.isupper()


def country_id(properties, taken):
    """ISO 3166-1 code, from ISO_A2_EH where Natural Earth leaves ISO_A2
    empty or not two letters (France, Norway, Kosovo, Taiwan), and the
    three-letter Natural Earth code where neither is free."""
    for key in ("ISO_A2", "ISO_A2_EH"):
        value = properties[key]
        if two_letters(value) and value not in taken:
            return value
    return properties["ADM0_A3"]


def main():
    directory = sys.argv[1] if len(sys.argv) > 1 else "/tmp/ne"
    countries = load(directory, COUNTRIES)
    regions = load(directory, REGIONS)

    taken = {c["properties"]["ISO_A2"] for c in countries if two_letters(c["properties"]["ISO_A2"])}
    features = []
    index = {}
    for country in sorted(countries, key=lambda c: c["properties"]["ADM0_A3"]):
        p = country["properties"]
        fid = p["ISO_A2"] if two_letters(p["ISO_A2"]) else country_id(p, taken)
        taken.add(fid)
        assert fid not in index, fid
        index[fid] = len(features)
        features.append(
            {
                "id": fid,
                "parent": 0xFFFF,
                "continent": CONTINENTS[p["REGION_UN"]],
                "names": names(p, True, NAME_OVERRIDES.get(fid, {})),
                "label": quantize(p["LABEL_X"], p["LABEL_Y"]),
                "geometry": country["geometry"],
            }
        )
    for region in sorted(regions, key=lambda r: r["properties"]["iso_3166_2"]):
        p = region["properties"]
        parent = index[p["iso_a2"]]
        fid = "".join(ch for ch in p["iso_3166_2"].replace("-", "_") if ch.isalnum() or ch == "_")
        assert fid not in index, fid
        index[fid] = len(features)
        features.append(
            {
                "id": fid,
                "parent": parent,
                "continent": features[parent]["continent"],
                "names": names(p, False, {}),
                "label": quantize(p["longitude"], p["latitude"]),
                "geometry": region["geometry"],
            }
        )

    out = bytearray(b"PWM1")
    out += struct.pack("<H", len(features))
    total_vertices = total_triangles = 0
    for feature in features:
        vertices, rings, triangles = shapes(feature["geometry"])
        total_vertices += len(vertices)
        total_triangles += len(triangles)
        fid = feature["id"].encode()
        out += struct.pack("<B", len(fid)) + fid
        out += struct.pack("<HB", feature["parent"], feature["continent"])
        for name in feature["names"]:
            encoded = name.encode()
            out += struct.pack("<H", len(encoded)) + encoded
        out += struct.pack("<HH", *feature["label"])
        out += struct.pack("<I", len(vertices))
        coords = np.array(vertices, dtype=np.int64).reshape(-1, 2)
        out += (np.diff(coords, axis=0, prepend=0) & 0xFFFF).astype("<u2").tobytes()
        out += struct.pack("<H", len(rings))
        for start, length in rings:
            out += struct.pack("<II", start, length)
        out += struct.pack("<I", len(triangles) * 3)
        indices = np.array(triangles, dtype=np.int64).reshape(-1)
        out += np.diff(indices, prepend=0).astype("<i4").tobytes()

    packed = zlib.compress(bytes(out), 9)
    with open(OUT, "wb") as file:
        file.write(packed)
    countries_count = sum(1 for f in features if f["parent"] == 0xFFFF)
    print(
        f"{countries_count} countries, {len(features) - countries_count} regions, "
        f"{total_vertices} vertices, {total_triangles} triangles, "
        f"{len(out)} bytes -> {len(packed)} bytes compressed"
    )


if __name__ == "__main__":
    main()
