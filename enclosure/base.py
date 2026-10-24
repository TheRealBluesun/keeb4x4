"""Parametric screw-down base for keeb-v2. Run with Python 3.11+.

Dimensions are millimetres. XY uses the PCB's front-left corner as origin;
+Y points toward USB, +Z up, and the printed underside is at Z=0.
The original KiCad file is read without modifying it.
"""

from pathlib import Path
import hashlib
import json
import math

import cadquery as cq
import sexpdata
import trimesh

HERE = Path(__file__).resolve().parent
PCB = HERE.parent / "keeb4x4.kicad_pcb"
OUT = HERE / "exports"

# Fit and construction parameters. Regenerate after changing any dimension.
EDGE_CLEARANCE = 0.35  # Per side, between PCB edge and case.
WALL = 2.4
FLOOR = 2.4
UNDERSIDE_CLEARANCE = 6.0  # Floor surface to PCB underside.
RIM_ABOVE_PCB = 0.4
POST_DIAMETER = 6.0
PILOT_DIAMETER = 2.6  # M3 screw forming its thread in plastic; no inserts.
PILOT_DEPTH = 7.0
SCREW_LENGTH = 8.0  # M3 x 8, measured from under the head, no washer.
USB_WIDTH = 16.0
USB_DROP = 3.5  # Bottom of open notch, below PCB top.
# R11 gets a local top notch; the lower post retains a complete screw wall.
POST_RELIEFS = {"H3": {"component": "R11", "clearance": 0.5,
                       "component_max_depth": 1.5}}
SCREENKEY_SIZE = (22.0, 25.3, 8.6)  # Waveshare keycap, NOT its carrier PCB.
SCREENKEY_URL = "https://docs.waveshare.com/0.85inch-ScreenKey-Module-W"


def children(node, kind):
    return [x for x in node if isinstance(x, list) and x and str(x[0]) == kind]


def child(node, kind):
    return next(iter(children(node, kind)), [])


def point_to_board(footprint, xy):
    at = child(footprint, "at")
    a = math.radians(at[3] if len(at) > 3 else 0)
    x, y = xy
    # KiCad positions have +Y down; footprint rotation is counterclockwise.
    return (at[1] + x * math.cos(a) + y * math.sin(a),
            at[2] - x * math.sin(a) + y * math.cos(a))


def read_board():
    text = PCB.read_text()
    board = sexpdata.loads(text)
    edges = [n for n in board if isinstance(n, list) and
             str(n[0]) in {"gr_line", "gr_arc"} and
             child(n, "layer")[1:] == ["Edge.Cuts"]]
    points = [child(e, k)[1:3] for e in edges for k in ("start", "end")]
    xmin, ymin = min(p[0] for p in points), min(p[1] for p in points)
    xmax, ymax = max(p[0] for p in points), max(p[1] for p in points)
    width, depth = xmax - xmin, ymax - ymin
    # This model deliberately supports the verified rounded rectangular board.
    # Stop instead of silently treating a future irregular outline as a rectangle.
    assert len(edges) == 8 and len(children(board, "gr_arc")) == 4
    assert (xmin, ymin, width, depth) == (0, 0, 100, 142.5)
    actual_ends = {tuple(p) for p in points}
    assert actual_ends == {(0, 3), (3, 0), (97, 0), (100, 3),
                           (100, 139.5), (97, 142.5), (3, 142.5), (0, 139.5)}

    def cad_point(p):
        return [round(p[0] - xmin, 6), round(ymax - p[1], 6)]

    mounts, switches, footprints, keepouts = [], [], {}, []
    for f in children(board, "footprint"):
        props = {p[1]: p[2] for p in children(f, "property")}
        ref = props["Reference"]
        layer = child(f, "layer")[1]
        pos = cad_point(child(f, "at")[1:3])
        footprints[ref] = {"position": pos, "value": props.get("Value", ""),
                           "footprint": f[1], "side": layer}
        pads = children(f, "pad")
        if ref.startswith("H"):
            assert len(pads) == 1 and child(pads[0], "drill")[1] == 3.2
            mounts.append({"ref": ref, "position": pos, "drill": 3.2,
                           "used": True})
            continue
        if ref.startswith("SW") and int(ref[2:]) <= 16:
            # MX footprint origin is a solder pad, not the key centre.
            centre_pad = next(p for p in pads if child(p, "drill")[1:] == [4])
            centre = cad_point(point_to_board(f, child(centre_pad, "at")[1:3]))
            switches.append({"ref": ref, "position": centre,
                             "display": int(ref[2:]) <= 4})
        # Conservative axis-aligned bottom component courtyard envelopes.
        courtyard = []
        for g in children(f, "fp_line") + children(f, "fp_rect"):
            if child(g, "layer")[1:] == ["B.CrtYd"]:
                a, b = child(g, "start")[1:3], child(g, "end")[1:3]
                for p in (a, b, (a[0], b[1]), (b[0], a[1])):
                    courtyard.append(cad_point(point_to_board(f, p)))
        if courtyard:
            keepouts.append({"ref": ref, "kind": "bottom_courtyard",
                             "bounds": [min(p[0] for p in courtyard),
                                        min(p[1] for p in courtyard),
                                        max(p[0] for p in courtyard),
                                        max(p[1] for p in courtyard)]})
        # Through-hole leads and switch pegs also occupy the bottom side.
        for pad in pads:
            if str(pad[2]) in {"thru_hole", "np_thru_hole"}:
                centre = cad_point(point_to_board(f, child(pad, "at")[1:3]))
                radius = max(child(pad, "size")[1:3]) / 2
                keepouts.append({"ref": ref, "kind": "lead_or_peg",
                                 "position": centre, "radius": radius})
    thickness = child(child(board, "general"), "thickness")[1]
    mounts.sort(key=lambda m: int(m["ref"][1:]))
    switches.sort(key=lambda m: int(m["ref"][2:]))
    assert len(mounts) == 8 and len(switches) == 16
    return {"pcb_source": "../keeb4x4.kicad_pcb",
            "pcb_sha256": hashlib.sha256(text.encode()).hexdigest(),
            "width": width, "depth": depth, "thickness": thickness,
            "corner_radius": 3.0, "mounts": mounts, "switches": switches,
            "footprints": footprints, "bottom_keepouts": keepouts,
            "screenkey_mm": SCREENKEY_SIZE, "screenkey_source": SCREENKEY_URL,
            "coordinates": "mm; origin PCB front-left; +Y toward USB; +Z up"}


def rounded_box(width, depth, height, radius, centre, z=0):
    return (cq.Workplane("XY").box(width, depth, height, centered=(True, True, False))
            .edges("|Z").fillet(radius).translate((*centre, z)))


def cylinder(x, y, radius, height, z):
    return cq.Workplane("XY").circle(radius).extrude(height).translate((x, y, z))


def post_relief(board, mount):
    """Open a padded component pocket into the bore, avoiding a thin web.

    The convex hull of the component clearance rectangle and the screw centre
    makes a continuous C-shaped bearing surface above a full circular shank.
    """
    config = POST_RELIEFS[mount["ref"]]
    k = next(k for k in board["bottom_keepouts"]
             if k["ref"] == config["component"] and k["kind"] == "bottom_courtyard")
    x0, y0, x1, y1 = k["bounds"]
    c = config["clearance"]
    points = sorted({tuple(mount["position"]), (x0-c, y0-c), (x1+c, y0-c),
                     (x1+c, y1+c), (x0-c, y1+c)})

    def cross(o, a, b):
        return (a[0]-o[0]) * (b[1]-o[1]) - (a[1]-o[1]) * (b[0]-o[0])

    chains = []
    for sequence in (points, points[::-1]):
        chain = []
        for p in sequence:
            while len(chain) >= 2 and cross(chain[-2], chain[-1], p) <= 0:
                chain.pop()
            chain.append(p)
        chains.append(chain[:-1])
    polygon = chains[0] + chains[1]
    depth = config["component_max_depth"] + c
    assert 0 < depth <= UNDERSIDE_CLEARANCE - 3.0
    bottom = FLOOR + UNDERSIDE_CLEARANCE - depth
    cut = (cq.Workplane("XY").polyline(polygon).close().extrude(depth + 0.1)
           .translate((0, 0, bottom)))
    details = {**config, "depth": depth, "bottom_z": bottom,
               "component_bounds": k["bounds"], "polygon": polygon}
    return cut, details


def build(board):
    w, d, r = board["width"], board["depth"], board["corner_radius"]
    centre = (w / 2, d / 2)
    pcb_bottom = FLOOR + UNDERSIDE_CLEARANCE
    pcb_top = pcb_bottom + board["thickness"]
    height = pcb_top + RIM_ABOVE_PCB
    margin = EDGE_CLEARANCE + WALL
    outer = rounded_box(w + 2 * margin, d + 2 * margin, height,
                        r + margin, centre)
    outer = outer.faces("<Z").edges().chamfer(0.4)
    outer = outer.faces(">Z").edges().chamfer(0.35)
    cavity = rounded_box(w + 2 * EDGE_CLEARANCE, d + 2 * EDGE_CLEARANCE,
                         height, r + EDGE_CLEARANCE, centre, FLOOR)
    base = outer.cut(cavity)
    # U-shaped cable notch: open at the top, rounded lower corners, no bridge.
    usb_x = board["footprints"]["J1"]["position"][0]
    usb_bottom = pcb_top - USB_DROP
    cutter = (cq.Workplane("XY").box(USB_WIDTH, 10, 30)
              .edges("|Y").fillet(2)
              .translate((usb_x, d + WALL / 2, usb_bottom + 15)))
    base = base.cut(cutter)

    assert 0 < PILOT_DIAMETER < POST_DIAMETER
    assert pcb_bottom - PILOT_DEPTH >= 1.0
    assert board["thickness"] < SCREW_LENGTH < PILOT_DEPTH + board["thickness"]
    reliefs = {}
    for m in board["mounts"]:
        if not m["used"]:
            continue
        x, y = m["position"]
        # A 45-degree foot strengthens the post below all component envelopes.
        foot = cq.Solid.makeCone(POST_DIAMETER / 2 + 1, POST_DIAMETER / 2,
                                 1.0, cq.Vector(x, y, FLOOR - 0.01))
        post = cylinder(x, y, POST_DIAMETER / 2, UNDERSIDE_CLEARANCE + 0.01,
                        FLOOR - 0.01)
        base = base.union(post).union(cq.Workplane(obj=foot))
        bore = cylinder(x, y, PILOT_DIAMETER / 2, PILOT_DEPTH + 0.02,
                        pcb_bottom - PILOT_DEPTH)
        mouth = cq.Solid.makeCone(PILOT_DIAMETER / 2, PILOT_DIAMETER / 2 + 0.3,
                                  0.3, cq.Vector(x, y, pcb_bottom - 0.3))
        base = base.cut(bore).cut(cq.Workplane(obj=mouth))
        if m["ref"] in POST_RELIEFS:
            relief, reliefs[m["ref"]] = post_relief(board, m)
            base = base.cut(relief)
    base = base.clean()
    # Cheap full-height front-left corner sample: one post, two walls and corner.
    clip = (cq.Workplane("XY").box(23, 23, height + 1, centered=(False, False, False))
            .translate((-margin, -margin, 0)))
    coupon = base.intersect(clip)
    pcb = rounded_box(w, d, board["thickness"], r, centre, pcb_bottom)
    for m in board["mounts"]:
        x, y = m["position"]
        pcb = pcb.cut(cylinder(x, y, m["drill"] / 2, board["thickness"] + 0.2,
                              pcb_bottom - 0.1))
    dims = {"outer_mm": [w + 2 * margin, d + 2 * margin, height],
            "pcb_bottom_z": pcb_bottom, "pcb_top_z": pcb_top,
            "pcb_side_clearance": EDGE_CLEARANCE, "floor_mm": FLOOR,
            "wall_mm": WALL, "underside_clearance_mm": UNDERSIDE_CLEARANCE,
            "post_diameter_mm": POST_DIAMETER, "pilot_diameter_mm": PILOT_DIAMETER,
            "pilot_depth_mm": PILOT_DEPTH, "screw_length_mm": SCREW_LENGTH,
            "usb_notch_width_mm": USB_WIDTH, "usb_notch_bottom_z": usb_bottom,
            "post_reliefs": reliefs}
    return base, coupon, pcb, dims


def distance_to_keepout(x, y, k):
    if "bounds" in k:
        x0, y0, x1, y1 = k["bounds"]
        return math.hypot(max(x0 - x, 0, x - x1), max(y0 - y, 0, y - y1))
    return math.dist((x, y), k["position"]) - k["radius"]


def validate(board, base, pcb):
    assert base.val().isValid(), "Invalid CAD solid"
    assert base.solids().size() == 1, "Disconnected printed part"
    overlap = base.val().intersect(pcb.val()).Volume()
    assert overlap < 1e-6, f"Case interferes with PCB: {overlap} mm3"
    clearances = []
    for m in board["mounts"]:
        x, y = m["position"]
        relieved_component = POST_RELIEFS.get(m["ref"], {}).get("component")
        other_keepouts = [k for k in board["bottom_keepouts"] if k["ref"] != relieved_component]
        closest = min(other_keepouts, key=lambda k: distance_to_keepout(x, y, k))
        gap = distance_to_keepout(x, y, closest) - POST_DIAMETER / 2
        relief_report = {}
        if m["used"]:
            assert gap >= 0.5, f"{m['ref']} post is too close to {closest['ref']}: {gap}"
            # Confirm both the 7 mm blind bore and supporting annulus survived booleans.
            top = FLOOR + UNDERSIDE_CLEARANCE
            bore_check = cylinder(x, y, PILOT_DIAMETER / 2 - 0.02,
                                   PILOT_DEPTH - 0.4, top - PILOT_DEPTH + 0.02)
            assert base.val().intersect(bore_check.val()).Volume() < 1e-6
            ring = cylinder(x, y, POST_DIAMETER / 2 - 0.02, 0.1, top - 0.2)
            ring = ring.cut(cylinder(x, y, PILOT_DIAMETER / 2 + 0.4, 0.3, top - 0.3))
            bearing = base.intersect(ring)
            if relieved_component:
                _, detail = post_relief(board, m)
                x0, y0, x1, y1 = detail["component_bounds"]
                height = detail["component_max_depth"]
                component = (cq.Workplane("XY").box(x1-x0, y1-y0, height,
                                                     centered=(False, False, False))
                             .translate((x0, y0, top-height)))
                clearance = base.val().distance(component.val())
                assert clearance >= detail["clearance"] - 1e-6
                assert base.val().intersect(component.val()).Volume() < 1e-6
                retained = bearing.val().Volume() / ring.val().Volume()
                assert retained >= 0.75 and bearing.solids().size() == 1
                lower_ring = ring.translate((0, 0, -detail["depth"]))
                assert abs(base.val().intersect(lower_ring.val()).Volume()
                           - lower_ring.val().Volume()) < 1e-6
                relief_report = {"component": relieved_component,
                                 "component_allowance_mm": height,
                                 "clearance_mm": round(clearance, 4),
                                 "notch_depth_mm": detail["depth"],
                                 "bearing_area_retained_percent": round(retained * 100, 2),
                                 "lower_post_full_annulus": True}
            else:
                assert abs(bearing.val().Volume() - ring.val().Volume()) < 1e-6
        clearances.append({"mount": m["ref"], "used": m["used"],
                           "nearest_unrelieved_component": closest["ref"],
                           "unrelieved_gap_mm": round(gap, 3), "relief": relief_report})
    return {"cad_valid": True, "one_solid": True, "pcb_overlap_mm3": overlap,
            "post_keepout_clearance": clearances,
            "limitations": ["No physical fit test yet",
                            "Bottom component heights and solder tails must fit within 6 mm",
                            "R11 including solder must project no more than 1.5 mm for 0.5 mm notch clearance",
                            "Preview keycaps, switch heights and control bodies are illustrative"]}


def main():
    OUT.mkdir(exist_ok=True)
    board = read_board()
    base, coupon, pcb, dims = build(board)
    report = validate(board, base, pcb)
    x, y = next(m["position"] for m in board["mounts"] if m["ref"] == "H3")
    h3_clip = (cq.Workplane("XY").box(16, 16, dims["outer_mm"][2] + 1,
                                      centered=(True, True, False)).translate((x, y, 0)))
    h3_sample = base.intersect(h3_clip)
    for name, part in [("keeb4x4_base", base), ("corner_fit_sample", coupon),
                       ("h3_fit_sample", h3_sample)]:
        assert part.val().isValid() and part.solids().size() == 1
        cq.exporters.export(part, str(OUT / f"{name}.step"))
        part.val().exportStl(str(OUT / f"{name}.stl"), tolerance=0.02,
                             angularTolerance=0.08, relative=False)
        mesh = trimesh.load_mesh(OUT / f"{name}.stl")
        assert mesh.is_watertight and mesh.is_winding_consistent and mesh.is_volume
        assert len(mesh.split()) == 1
        report[name] = {"watertight": True, "one_mesh": True,
                        "bounds_mm": mesh.bounds.tolist(),
                        "volume_mm3": round(float(mesh.volume), 2),
                        "triangles": len(mesh.faces)}
    (OUT / "mechanical.json").write_text(json.dumps({**board, "base": dims}, indent=2) + "\n")
    (OUT / "validation.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"dimensions_mm": dims["outer_mm"], "validation": report}, indent=2))


if __name__ == "__main__":
    main()
