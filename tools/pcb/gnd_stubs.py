"""Give every SK6812 GND pad its own via down to the In1 GND plane.

Run BEFORE route_chain.py: the data-chain router then sees these as obstacles
and won't horseshoe around a GND pad and strand it from the pour.
"""
import os, sys, math
import pcbnew
from led_router import Obstacles, GRID, VIA_D, CLEAR, mm, FM, seg_dist

BOARD = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'keeb4x4.kicad_pcb')
b = pcbnew.LoadBoard(BOARD)
gnd = b.GetNetsByName()['GND'].GetNetCode()
obs = Obstacles(b)

in1 = None
for z in b.Zones():
    if z.GetNetCode() == gnd and z.IsOnLayer(pcbnew.In1_Cu):
        in1 = z.GetFilledPolysList(pcbnew.In1_Cu)

allpads = [(p, fp) for fp in b.GetFootprints() for p in fp.Pads()]

# candidate offsets from pad1, nearest first, biased away from the LED body (west/north)
OFFS = []
for r in (1.30, 1.55, 1.80, 2.10, 2.45, 2.85, 3.30, 3.80):
    # west/north first (away from the LED body and its cutout), then all round
    for deg in (180, 200, 160, 225, 135, 250, 110, 270, 90,
                290, 70, 315, 45, 335, 25, 350, 10, 0):
        OFFS.append((r * math.cos(math.radians(deg)), r * math.sin(math.radians(deg))))

ok, bad = 0, []
for i in range(1, 17):
    fp = b.FindFootprintByReference(f'LED{i}')
    p1 = next(p for p in fp.Pads() if p.GetNumber() == '1')
    q = p1.GetPosition()
    px, py = mm(q.x), mm(q.y)
    win = 6.0
    x0, y0 = px - win, py - win
    n = int(2 * win / GRID) + 2
    blocked, vblock = obs.grid(x0, y0, n, n, gnd, 0.15)
    placed = None
    for dx, dy in OFFS:
        vx, vy = px + dx, py + dy
        pt = pcbnew.VECTOR2I(FM(vx), FM(vy))
        ix, iy = int(round((vx - x0) / GRID)), int(round((vy - y0) / GRID))
        if not (0 <= ix < n and 0 <= iy < n) or vblock[iy][ix]:
            continue
        if in1 is not None and not in1.Collide(pt, 0):
            continue
        # must not sit inside any pad (via-in-pad wicks solder at reflow)
        if any(p.HitTest(pt) for p, _ in allpads):
            continue
        # the stub itself must be clear on B.Cu
        clear = True
        steps = max(2, int(math.hypot(dx, dy) / GRID))
        for s in range(steps + 1):
            tx, ty = px + dx * s / steps, py + dy * s / steps
            jx, jy = int(round((tx - x0) / GRID)), int(round((ty - y0) / GRID))
            if not (0 <= jx < n and 0 <= jy < n) or blocked[1][jy][jx]:
                clear = False
                break
        if not clear:
            continue
        placed = (vx, vy)
        break
    if not placed:
        bad.append(f'LED{i}')
        print(f'  LED{i:<3} no clear GND via spot')
        continue
    vx, vy = placed
    t = pcbnew.PCB_TRACK(b)
    t.SetStart(q); t.SetEnd(pcbnew.VECTOR2I(FM(vx), FM(vy)))
    t.SetWidth(FM(0.30)); t.SetLayer(pcbnew.B_Cu); t.SetNetCode(gnd); b.Add(t)
    v = pcbnew.PCB_VIA(b)
    v.SetPosition(pcbnew.VECTOR2I(FM(vx), FM(vy)))
    v.SetWidth(FM(VIA_D)); v.SetDrill(FM(0.3))
    v.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu); v.SetNetCode(gnd); b.Add(v)
    obs.segs.append((pcbnew.B_Cu, px, py, vx, vy, 0.15, gnd))
    obs.circles.append((None, vx, vy, VIA_D / 2, gnd, 'via'))
    ok += 1
    print(f'  LED{i:<3} GND via at ({vx:6.2f},{vy:7.2f})  stub {math.hypot(vx-px, vy-py):.2f}mm')

print(f'\nplaced {ok}/16 GND stub vias; failed: {bad}')
pcbnew.ZONE_FILLER(b).Fill(b.Zones())
pcbnew.SaveBoard(BOARD, b)
print('SAVED')
