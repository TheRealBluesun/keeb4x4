"""Grid A* router for the SK6812 data chain on keeb4x4.

Rasterises every other net's copper (tracks, vias, pads) plus the board/LED
cutouts into a per-layer blocked grid, then searches F.Cu+B.Cu with via
transitions. Emits merged collinear segments so no corner-cutting.
"""
import heapq, math, os
import pcbnew

BOARD = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'keeb4x4.kicad_pcb')
GRID = 0.25          # mm
CLEAR = 0.22         # mm design clearance (rule is 0.20, small margin)
TRACK_W = 0.20
VIA_D = 0.60
VIA_COST = 14.0
LAYERS = [pcbnew.F_Cu, pcbnew.B_Cu]

mm = pcbnew.ToMM
FM = pcbnew.FromMM


def seg_dist(px, py, x1, y1, x2, y2):
    dx, dy = x2 - x1, y2 - y1
    L2 = dx * dx + dy * dy
    if L2 == 0:
        return math.hypot(px - x1, py - y1)
    t = max(0.0, min(1.0, ((px - x1) * dx + (py - y1) * dy) / L2))
    return math.hypot(px - (x1 + t * dx), py - (y1 + t * dy))


class Obstacles:
    """Collect copper + cutouts once; rasterise per-route."""

    def __init__(self, board):
        self.b = board
        self.segs = []    # (layer|None for all, x1,y1,x2,y2, halfwidth, netcode)
        self.circles = []  # (layer|None, cx, cy, radius, netcode)
        for t in board.GetTracks():
            n = t.GetNetCode()
            if t.GetClass() == 'PCB_VIA':
                p = t.GetPosition()
                self.circles.append((None, mm(p.x), mm(p.y), mm(t.GetWidth()) / 2, n, 'via'))
            else:
                s, e = t.GetStart(), t.GetEnd()
                self.segs.append((t.GetLayer(), mm(s.x), mm(s.y), mm(e.x), mm(e.y),
                                  mm(t.GetWidth()) / 2, n))
        # NB: PAD::GetLayer() lies for pads on flipped footprints (returns F.Cu
        # for a B.Cu pad). Must use the layer SET.
        ALLCU = (pcbnew.F_Cu, pcbnew.In1_Cu, pcbnew.In2_Cu, pcbnew.B_Cu)
        for fp in board.GetFootprints():
            for p in fp.Pads():
                q = p.GetPosition()
                sz = p.GetSize()
                r = math.hypot(mm(sz.x), mm(sz.y)) / 2
                ls = p.GetLayerSet()
                on = frozenset(l for l in ALLCU if ls.Contains(l))
                kind = 'pad_smd' if p.GetAttribute() == pcbnew.PAD_ATTRIB_SMD else 'pad_tht'
                self.circles.append((on or None, mm(q.x), mm(q.y), r, p.GetNetCode(), kind))
        # board + footprint Edge.Cuts -> hard obstacle on every layer
        self.cuts = []
        for d in board.GetDrawings():
            if board.GetLayerName(d.GetLayer()) == 'Edge.Cuts':
                self._add_cut(d)
        for fp in board.GetFootprints():
            for d in fp.GraphicalItems():
                if board.GetLayerName(d.GetLayer()) == 'Edge.Cuts':
                    self._add_cut(d)

    def _add_cut(self, d):
        """Edge.Cuts -> polyline segments. Arcs/circles are sampled: taking only
        GetStart()/GetEnd() on an arc misses the entire bulge."""
        try:
            kind = d.GetShapeStr()
        except Exception:
            kind = 'Line'
        try:
            if kind in ('Arc', 'Circle'):
                c = d.GetCenter()
                cx, cy = mm(c.x), mm(c.y)
                s = d.GetStart()
                r = math.hypot(mm(s.x) - cx, mm(s.y) - cy)
                if kind == 'Circle':
                    a0, sweep = 0.0, 2 * math.pi
                else:
                    e = d.GetEnd()
                    a0 = math.atan2(mm(s.y) - cy, mm(s.x) - cx)
                    a1 = math.atan2(mm(e.y) - cy, mm(e.x) - cx)
                    sweep = a1 - a0
                    while sweep <= -math.pi:
                        sweep += 2 * math.pi
                    while sweep > math.pi:
                        sweep -= 2 * math.pi
                n = max(4, int(abs(sweep) * r / 0.15) + 1)
                pts = [(cx + r * math.cos(a0 + sweep * i / n),
                        cy + r * math.sin(a0 + sweep * i / n)) for i in range(n + 1)]
                for p, q in zip(pts, pts[1:]):
                    self.cuts.append((p[0], p[1], q[0], q[1]))
                return
            if kind in ('Rect',):
                s, e = d.GetStart(), d.GetEnd()
                x1, y1, x2, y2 = mm(s.x), mm(s.y), mm(e.x), mm(e.y)
                for a in ((x1, y1, x2, y1), (x2, y1, x2, y2),
                          (x2, y2, x1, y2), (x1, y2, x1, y1)):
                    self.cuts.append(a)
                return
            if kind in ('Polygon',):
                sp = d.GetPolyShape()
                ol = sp.Outline(0)
                pts = [(mm(ol.CPoint(i).x), mm(ol.CPoint(i).y)) for i in range(ol.PointCount())]
                for p, q in zip(pts, pts[1:] + pts[:1]):
                    self.cuts.append((p[0], p[1], q[0], q[1]))
                return
            s, e = d.GetStart(), d.GetEnd()
            self.cuts.append((mm(s.x), mm(s.y), mm(e.x), mm(e.y)))
        except Exception:
            pass

    def grid(self, x0, y0, nx, ny, netcode, halfw):
        """(blocked[layer_idx][iy][ix], via_blocked[iy][ix]) for the window.

        via_blocked is computed with the via radius (not the track halfwidth)
        and against copper on EVERY layer, since a through via punches all four.
        """
        pad = CLEAR + halfw
        vpad = CLEAR + VIA_D / 2
        blocked = [[[False] * nx for _ in range(ny)] for _ in LAYERS]
        vblock = [[False] * nx for _ in range(ny)]
        x1, y1 = x0 + nx * GRID, y0 + ny * GRID

        def stamp(grid2d, ax, ay, bx, by, r):
            if max(ax, bx) < x0 - r - 1 or min(ax, bx) > x1 + r + 1:
                return
            if max(ay, by) < y0 - r - 1 or min(ay, by) > y1 + r + 1:
                return
            for iy in range(max(0, int((min(ay, by) - r - y0) / GRID)),
                            min(ny, int((max(ay, by) + r - y0) / GRID) + 2)):
                py = y0 + iy * GRID
                for ix in range(max(0, int((min(ax, bx) - r - x0) / GRID)),
                                min(nx, int((max(ax, bx) + r - x0) / GRID) + 2)):
                    if not grid2d[iy][ix] and seg_dist(x0 + ix * GRID, py, ax, ay, bx, by) < r:
                        grid2d[iy][ix] = True

        # --- via grid: every layer's copper, via radius ---
        for (slay, ax, ay, bx, by, hw, net) in self.segs:
            if net == netcode:
                continue
            stamp(vblock, ax, ay, bx, by, hw + vpad)
        for (clay, cx, cy, rad, net, kind) in self.circles:
            if net == netcode:
                # same net needs no clearance, but a via must never sit in an
                # SMD pad (solder wicks down the barrel during reflow)
                if kind == 'pad_smd':
                    stamp(vblock, cx, cy, cx, cy, rad)
                continue
            stamp(vblock, cx, cy, cx, cy, rad + vpad)
        for (ax, ay, bx, by) in self.cuts:
            stamp(vblock, ax, ay, bx, by, 0.10 + VIA_D / 2)
        for li, lay in enumerate(LAYERS):
            B = blocked[li]
            for (slay, ax, ay, bx, by, hw, net) in self.segs:
                if slay != lay or net == netcode:
                    continue
                if max(ax, bx) < x0 - 2 or min(ax, bx) > x1 + 2:
                    continue
                if max(ay, by) < y0 - 2 or min(ay, by) > y1 + 2:
                    continue
                r = hw + pad
                for iy in range(max(0, int((min(ay, by) - r - y0) / GRID)),
                                min(ny, int((max(ay, by) + r - y0) / GRID) + 2)):
                    py = y0 + iy * GRID
                    for ix in range(max(0, int((min(ax, bx) - r - x0) / GRID)),
                                    min(nx, int((max(ax, bx) + r - x0) / GRID) + 2)):
                        if not B[iy][ix] and seg_dist(x0 + ix * GRID, py, ax, ay, bx, by) < r:
                            B[iy][ix] = True
            for (clay, cx, cy, rad, net, kind) in self.circles:
                if net == netcode or (clay is not None and lay not in clay):
                    continue
                r = rad + pad
                if cx < x0 - r - 1 or cx > x1 + r + 1 or cy < y0 - r - 1 or cy > y1 + r + 1:
                    continue
                for iy in range(max(0, int((cy - r - y0) / GRID)),
                                min(ny, int((cy + r - y0) / GRID) + 2)):
                    py = y0 + iy * GRID
                    for ix in range(max(0, int((cx - r - x0) / GRID)),
                                    min(nx, int((cx + r - x0) / GRID) + 2)):
                        if not B[iy][ix] and math.hypot(x0 + ix * GRID - cx, py - cy) < r:
                            B[iy][ix] = True
            for (ax, ay, bx, by) in self.cuts:
                r = 0.10 + halfw          # min_copper_edge_clearance
                if max(ax, bx) < x0 - 2 or min(ax, bx) > x1 + 2:
                    continue
                if max(ay, by) < y0 - 2 or min(ay, by) > y1 + 2:
                    continue
                for iy in range(max(0, int((min(ay, by) - r - y0) / GRID)),
                                min(ny, int((max(ay, by) + r - y0) / GRID) + 2)):
                    py = y0 + iy * GRID
                    for ix in range(max(0, int((min(ax, bx) - r - x0) / GRID)),
                                    min(nx, int((max(ax, bx) + r - x0) / GRID) + 2)):
                        if not B[iy][ix] and seg_dist(x0 + ix * GRID, py, ax, ay, bx, by) < r:
                            B[iy][ix] = True
        return blocked, vblock


NB = [(1, 0, 1.0), (-1, 0, 1.0), (0, 1, 1.0), (0, -1, 1.0),
      (1, 1, 1.414), (1, -1, 1.414), (-1, 1, 1.414), (-1, -1, 1.414)]


def astar(blocked, nx, ny, start, goal, vblock=None):
    """start/goal = (ix, iy, layer_idx). Returns list of nodes or None."""
    sx, sy, sl = start
    gx, gy, gl = goal
    def h(x, y):
        return (abs(x - gx) + abs(y - gy)) * 0.85
    openq = [(h(sx, sy), 0.0, start, None)]
    came, best = {}, {start: 0.0}
    while openq:
        f, g, cur, prev = heapq.heappop(openq)
        if cur in came:
            continue
        came[cur] = prev
        cx, cy, cl = cur
        if (cx, cy) == (gx, gy) and cl == gl:
            path, n = [], cur
            while n is not None:
                path.append(n)
                n = came[n]
            return path[::-1]
        for dx, dy, w in NB:
            nx_, ny_ = cx + dx, cy + dy
            if not (0 <= nx_ < nx and 0 <= ny_ < ny):
                continue
            if blocked[cl][ny_][nx_]:
                continue
            if dx and dy and (blocked[cl][cy][nx_] or blocked[cl][ny_][cx]):
                continue          # no diagonal squeeze
            nn = (nx_, ny_, cl)
            ng = g + w
            if ng < best.get(nn, 1e18):
                best[nn] = ng
                heapq.heappush(openq, (ng + h(nx_, ny_), ng, nn, cur))
        ol = 1 - cl
        if not blocked[ol][cy][cx] and not (vblock and vblock[cy][cx]):
            nn = (cx, cy, ol)
            ng = g + VIA_COST
            if ng < best.get(nn, 1e18):
                best[nn] = ng
                heapq.heappush(openq, (ng + h(cx, cy), ng, nn, cur))
    return None


def emit(path, x0, y0):
    """Collapse a node path into (layer_idx, pts) runs + via positions."""
    runs, vias = [], []
    cur_layer, pts = path[0][2], [(path[0][0], path[0][1])]
    for n in path[1:]:
        if n[2] != cur_layer:
            runs.append((cur_layer, pts))
            vias.append((x0 + n[0] * GRID, y0 + n[1] * GRID))
            cur_layer, pts = n[2], [(n[0], n[1])]
        else:
            pts.append((n[0], n[1]))
    runs.append((cur_layer, pts))
    out = []
    for lay, p in runs:
        if len(p) < 2:
            continue
        merged = [p[0]]
        for i in range(1, len(p) - 1):
            ax, ay = merged[-1]
            bx, by = p[i]
            cx, cy = p[i + 1]
            if (bx - ax) * (cy - by) != (by - ay) * (cx - bx):
                merged.append(p[i])
        merged.append(p[-1])
        out.append((lay, [(x0 + a * GRID, y0 + b * GRID) for a, b in merged]))
    return out, vias
