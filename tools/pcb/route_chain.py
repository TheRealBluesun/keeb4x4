"""Route the SK6812 data chain + the two awkward VBUS stubs."""
import sys, math
import pcbnew
from led_router import *

b = pcbnew.LoadBoard(BOARD)
nets = b.GetNetsByName()


def pad(ref, num):
    f = b.FindFootprintByReference(ref)
    p = next(p for p in f.Pads() if p.GetNumber() == num)
    q = p.GetPosition()
    return mm(q.x), mm(q.y)


drop = []
for ref in ('LED3', 'LED4'):
    sx, sy = pad(ref, '3')
    for t in b.GetTracks():
        if t.GetNetname() == 'VBUS' and t.GetClass() == 'PCB_TRACK':
            s = t.GetStart()
            if abs(mm(s.x) - sx) < 0.01 and abs(mm(s.y) - sy) < 0.01:
                drop.append(t)
print(f'dropping {len(drop)} conflicting VBUS stubs')
for t in drop:
    b.Remove(t)

obs = Obstacles(b)
tasks = [('VBUS', pad('LED3', '3'), pad('C11', '1'), 0.30),
         ('VBUS', pad('LED4', '3'), pad('C12', '1'), 0.30),
         ('LED_DATA', pad('U4', '4'), pad('LED1', '2'), TRACK_W)]
for i in range(1, 16):
    tasks.append((f'LED_D{i}', pad(f'LED{i}', '4'), pad(f'LED{i+1}', '2'), TRACK_W))

BCU = LAYERS.index(pcbnew.B_Cu)
ok, fail = 0, []
for name, (sx, sy), (gx, gy), W in tasks:
    net = nets[name].GetNetCode()
    path = None
    for m in (10.0, 18.0, 30.0):        # widen the search window on failure
        x0, y0 = min(sx, gx) - m, min(sy, gy) - m
        nx = int((abs(gx - sx) + 2 * m) / GRID) + 2
        ny = int((abs(gy - sy) + 2 * m) / GRID) + 2
        blocked, vblock = obs.grid(x0, y0, nx, ny, net, W / 2)
        si = (int(round((sx - x0) / GRID)), int(round((sy - y0) / GRID)), BCU)
        gi = (int(round((gx - x0) / GRID)), int(round((gy - y0) / GRID)), BCU)
        for c in (si, gi):
            blocked[c[2]][c[1]][c[0]] = False
        path = astar(blocked, nx, ny, si, gi, vblock)
        if path:
            break
    if not path:
        fail.append(name)
        print(f'  {name:10} FAILED')
        continue
    runs, vias = emit(path, x0, y0)
    runs = [r for r in runs if len(r[1]) >= 2]
    runs[0][1][0] = (sx, sy)
    runs[-1][1][-1] = (gx, gy)
    L = 0
    for li, pts in runs:
        for a, c in zip(pts, pts[1:]):
            t = pcbnew.PCB_TRACK(b)
            t.SetStart(pcbnew.VECTOR2I(FM(a[0]), FM(a[1])))
            t.SetEnd(pcbnew.VECTOR2I(FM(c[0]), FM(c[1])))
            t.SetWidth(FM(W)); t.SetLayer(LAYERS[li]); t.SetNetCode(net); b.Add(t)
            obs.segs.append((LAYERS[li], a[0], a[1], c[0], c[1], W / 2, net))
            L += math.hypot(c[0] - a[0], c[1] - a[1])
    for vx, vy in vias:
        v = pcbnew.PCB_VIA(b)
        v.SetPosition(pcbnew.VECTOR2I(FM(vx), FM(vy)))
        v.SetWidth(FM(VIA_D)); v.SetDrill(FM(0.3))
        v.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu); v.SetNetCode(net); b.Add(v)
        obs.circles.append((None, vx, vy, VIA_D / 2, net, 'via'))
    ok += 1
    print(f'  {name:10} {L:6.2f}mm  {sum(len(p)-1 for _, p in runs):2} segs  {len(vias)} vias')

print(f'\nrouted {ok}/{len(tasks)}  failed: {fail}')
pcbnew.ZONE_FILLER(b).Fill(b.Zones())
pcbnew.SaveBoard(BOARD, b)
print('SAVED')
