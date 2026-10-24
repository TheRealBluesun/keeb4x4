"""Add GND stitching vias so every GND pour region reaches the In1 plane."""
import os, sys, math
import pcbnew
from led_router import Obstacles, GRID, VIA_D, CLEAR, mm, FM

BOARD = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'keeb4x4.kicad_pcb')
b = pcbnew.LoadBoard(BOARD)
gnd = b.GetNetsByName()['GND'].GetNetCode()
obs = Obstacles(b)


def bridges():
    out = []
    for t in b.GetTracks():
        if t.GetNetCode() == gnd and t.GetClass() == 'PCB_VIA':
            out.append(t.GetPosition())
    for fp in b.GetFootprints():
        for p in fp.Pads():
            if p.GetNetCode() == gnd and p.GetAttribute() != pcbnew.PAD_ATTRIB_SMD:
                out.append(p.GetPosition())
    return out


in1 = None
for z in b.Zones():
    if z.GetNetCode() == gnd and z.IsOnLayer(pcbnew.In1_Cu):
        in1 = z.GetFilledPolysList(pcbnew.In1_Cu)

added = 0
for z in b.Zones():
    if z.GetNetCode() != gnd:
        continue
    for lay in (pcbnew.F_Cu, pcbnew.B_Cu):
        if not z.IsOnLayer(lay):
            continue
        poly = z.GetFilledPolysList(lay)
        for i in range(poly.OutlineCount()):
            if any(poly.Contains(p, i) for p in bridges()):
                continue
            bb = poly.Outline(i).BBox()
            x0, y0 = mm(bb.GetLeft()) - 1, mm(bb.GetTop()) - 1
            nx = int((mm(bb.GetRight()) - mm(bb.GetLeft()) + 2) / GRID) + 2
            ny = int((mm(bb.GetBottom()) - mm(bb.GetTop()) + 2) / GRID) + 2
            _, vblock = obs.grid(x0, y0, nx, ny, gnd, VIA_D / 2)
            # prefer points far from the island edge -> scan centre-out
            cx = (mm(bb.GetLeft()) + mm(bb.GetRight())) / 2
            cy = (mm(bb.GetTop()) + mm(bb.GetBottom())) / 2
            cands = []
            for iy in range(ny):
                for ix in range(nx):
                    if vblock[iy][ix]:
                        continue
                    px, py = x0 + ix * GRID, y0 + iy * GRID
                    pt = pcbnew.VECTOR2I(FM(px), FM(py))
                    if not poly.Contains(pt, i):
                        continue
                    if in1 is not None and not in1.Collide(pt, 0):
                        continue
                    cands.append((math.hypot(px - cx, py - cy), px, py))
            if not cands:
                print(f'  {b.GetLayerName(lay)} #{i}: NO ROOM for a stitch via')
                continue
            cands.sort()
            _, px, py = cands[0]
            v = pcbnew.PCB_VIA(b)
            v.SetPosition(pcbnew.VECTOR2I(FM(px), FM(py)))
            v.SetWidth(FM(VIA_D)); v.SetDrill(FM(0.3))
            v.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu)
            v.SetNetCode(gnd); b.Add(v)
            obs.circles.append((None, px, py, VIA_D / 2, gnd, 'via'))
            added += 1
            print(f'  {b.GetLayerName(lay)} #{i}: stitch via at ({px:.2f},{py:.2f})')

print(f'\nadded {added} GND stitching vias')
pcbnew.ZONE_FILLER(b).Fill(b.Zones())
pcbnew.SaveBoard(BOARD, b)
print('SAVED')
