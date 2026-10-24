"""Render the actual base plus illustrative components, and a dimensioned plan."""

import math

import cadquery as cq
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.patches import Circle, FancyBboxPatch, Polygon, Rectangle
import vtk

from base import OUT, SCREENKEY_SIZE, build, cylinder, read_board, rounded_box


def vtk_actor(part, colour):
    vertices, triangles = part.val().tessellate(0.08, 0.15)
    points, cells = vtk.vtkPoints(), vtk.vtkCellArray()
    for p in vertices:
        points.InsertNextPoint(p.x, p.y, p.z)
    for t in triangles:
        cells.InsertNextCell(3)
        for i in t:
            cells.InsertCellPoint(i)
    poly = vtk.vtkPolyData()
    poly.SetPoints(points)
    poly.SetPolys(cells)
    normals = vtk.vtkPolyDataNormals()
    normals.SetInputData(poly)
    normals.SetFeatureAngle(40)
    mapper = vtk.vtkPolyDataMapper()
    mapper.SetInputConnection(normals.GetOutputPort())
    actor = vtk.vtkActor()
    actor.SetMapper(mapper)
    p = actor.GetProperty()
    p.SetColor(*colour)
    p.SetAmbient(0.27)
    p.SetDiffuse(0.73)
    p.SetSpecular(0.15)
    p.SetSpecularPower(35)
    p.SetInterpolationToPhong()
    return actor


def label(renderer, text, xy, size, colour=(0.18, 0.23, 0.26)):
    actor = vtk.vtkTextActor()
    actor.SetInput(text)
    actor.GetPositionCoordinate().SetCoordinateSystemToNormalizedViewport()
    actor.SetPosition(*xy)
    prop = actor.GetTextProperty()
    prop.SetFontFamilyToArial()
    prop.SetFontSize(size)
    prop.SetColor(*colour)
    renderer.AddViewProp(actor)


def render(board, base, pcb, dims):
    window = vtk.vtkRenderWindow()
    window.SetOffScreenRendering(1)
    window.SetSize(1800, 1050)
    window.SetMultiSamples(8)
    for side in range(2):
        renderer = vtk.vtkRenderer()
        renderer.SetViewport(side / 2, 0, (side + 1) / 2, 1)
        renderer.SetBackground(0.96, 0.96, 0.94)
        window.AddRenderer(renderer)
        renderer.AddActor(vtk_actor(base, (0.19, 0.27, 0.29)))
        if side:
            renderer.AddActor(vtk_actor(pcb, (0.21, 0.39, 0.30)))
            top = dims["pcb_top_z"]
            for m in board["mounts"]:
                if m["used"]:
                    x, y = m["position"]
                    screw = cylinder(x, y, 2.7, 2, top)
                    socket = (cq.Workplane("XY").polygon(6, 2.6).extrude(1)
                              .translate((x, y, top + 1.2)))
                    renderer.AddActor(vtk_actor(screw.cut(socket), (0.56, 0.58, 0.60)))
            for key in board["switches"]:
                x, y = key["position"]
                switch = rounded_box(12.7, 12.7, 6, 0.8, (x, y), top)
                renderer.AddActor(vtk_actor(switch, (0.22, 0.23, 0.23)))
                w, d, h = SCREENKEY_SIZE if key["display"] else (18, 18, 7.5)
                cap = rounded_box(w, d, h, 1.4, (x, y), top + 6)
                cap = cap.faces(">Z").edges().fillet(0.8)
                renderer.AddActor(vtk_actor(cap, (0.11, 0.14, 0.16) if key["display"]
                                           else (0.82, 0.81, 0.75)))
                if key["display"]:
                    screen = rounded_box(15.21, 15.21, 0.15, 0.7, (x, y + 1.5),
                                         top + 6 + h)
                    renderer.AddActor(vtk_actor(screen, (0.22, 0.62, 0.67)))
            # Show all fitted FPC connectors; only the top row has screen caps.
            for number in range(4, 16):
                x, y = board["footprints"][f"J{number}"]["position"]
                connector = rounded_box(9, 3.8, 1.8, 0.3, (x, y), top)
                renderer.AddActor(vtk_actor(connector, (0.66, 0.62, 0.49)))
            # EC11's shaft centre is offset from its footprint origin.
            enc_x, enc_y = board["footprints"]["SW17"]["position"]
            enc_x, enc_y = enc_x + 7.5, enc_y - 2.5
            encoder = rounded_box(12, 11.6, 6, 0.5, (enc_x, enc_y), top)
            renderer.AddActor(vtk_actor(encoder, (0.65, 0.67, 0.66)))
            renderer.AddActor(vtk_actor(cylinder(enc_x, enc_y, 3, 20, top),
                                       (0.72, 0.74, 0.74)))
            usb_x, usb_y = board["footprints"]["J1"]["position"]
            renderer.AddActor(vtk_actor(rounded_box(8.94, 7.3, 3.2, 0.4,
                                                    (usb_x, usb_y), top),
                                       (0.65, 0.67, 0.69)))
            for ref, pins in [("J2", 4), ("J3", 5)]:
                x, y = board["footprints"][ref]["position"]
                header = rounded_box(pins * 2.54, 2.54, 2.5, 0.2,
                                     (x + (pins - 1) * 1.27, y), top)
                renderer.AddActor(vtk_actor(header, (0.16, 0.17, 0.17)))
        camera = renderer.GetActiveCamera()
        camera.SetPosition(230, -205, 290)
        camera.SetFocalPoint(50, 71.25, 5 if side == 0 else 11)
        camera.SetViewUp(0, 0, 1)
        camera.ParallelProjectionOn()
        camera.SetParallelScale(114)
        renderer.ResetCameraClippingRange()
        title = "01  /  PRINTED BASE" if side == 0 else "02  /  BOARD + KEY LAYOUT"
        sub = "105.5 x 148 x 10.4 mm" if side == 0 else "4 display keys + 12 conventional keys"
        label(renderer, title, (0.07, 0.90), 28)
        label(renderer, sub, (0.07, 0.85), 22, (0.40, 0.46, 0.47))
        detail = ("Eight M3 mounts / H3 notched for R11\n6 mm below PCB / 0.35 mm edge clearance"
                  if side == 0 else
                  "Component shapes illustrate the layout.\nKeycap profiles and installed heights are approximate.")
        label(renderer, detail, (0.07, 0.09), 20, (0.40, 0.46, 0.47))
    window.Render()
    grab = vtk.vtkWindowToImageFilter()
    grab.SetInput(window)
    grab.SetInputBufferTypeToRGB()
    grab.ReadFrontBufferOff()
    grab.Update()
    writer = vtk.vtkPNGWriter()
    writer.SetFileName(str(OUT / "preview.png"))
    writer.SetInputConnection(grab.GetOutputPort())
    writer.Write()
    window.Finalize()


def relief_detail(board, base, dims):
    """Show the actual post with the checked resistor clearance envelope."""
    x, y = next(m["position"] for m in board["mounts"] if m["ref"] == "H3")
    detail = dims["post_reliefs"]["H3"]
    clip = (cq.Workplane("XY").box(12, 12, 12, centered=(True, True, False))
            .translate((x, y, 0)))
    window = vtk.vtkRenderWindow()
    window.SetOffScreenRendering(1)
    window.SetSize(1000, 900)
    window.SetMultiSamples(8)
    renderer = vtk.vtkRenderer()
    renderer.SetBackground(0.96, 0.96, 0.94)
    window.AddRenderer(renderer)
    renderer.AddActor(vtk_actor(base.intersect(clip), (0.24, 0.36, 0.39)))
    x0, y0, x1, y1 = detail["component_bounds"]
    height = detail["component_max_depth"]
    envelope = (cq.Workplane("XY").box(x1-x0, y1-y0, height,
                                       centered=(False, False, False))
                .translate((x0, y0, dims["pcb_bottom_z"] - height)))
    actor = vtk_actor(envelope, (0.87, 0.53, 0.20))
    actor.GetProperty().SetOpacity(0.65)
    renderer.AddActor(actor)
    camera = renderer.GetActiveCamera()
    camera.SetPosition(x + 20, y + 25, 25)
    camera.SetFocalPoint(x, y, 4.6)
    camera.SetViewUp(0, 0, 1)
    camera.ParallelProjectionOn()
    camera.SetParallelScale(12)
    renderer.ResetCameraClippingRange()
    label(renderer, "H3  /  RESISTOR CLEARANCE", (0.07, 0.91), 28)
    label(renderer, "2 mm top notch / full round post below", (0.07, 0.86), 21)
    label(renderer, "Amber: R11 courtyard with 1.5 mm depth allowance.\n"
          "0.5 mm clearance to the post. PCB omitted to show the notch.",
          (0.07, 0.07), 20, (0.40, 0.46, 0.47))
    window.Render()
    grab = vtk.vtkWindowToImageFilter()
    grab.SetInput(window)
    grab.ReadFrontBufferOff()
    grab.Update()
    writer = vtk.vtkPNGWriter()
    writer.SetFileName(str(OUT / "h3_relief_detail.png"))
    writer.SetInputConnection(grab.GetOutputPort())
    writer.Write()
    window.Finalize()


def drawing(board, dims):
    plt.rcParams.update({"font.family": "DejaVu Sans", "font.size": 9})
    fig, ax = plt.subplots(figsize=(8.3, 11.7))
    w, d = board["width"], board["depth"]
    outer_w, outer_d, _ = dims["outer_mm"]
    ax.add_patch(FancyBboxPatch((-(outer_w - w) / 2, -(outer_d - d) / 2),
                                outer_w, outer_d, boxstyle="round,pad=0,rounding_size=5.75",
                                facecolor="#e0e7e5", edgecolor="#374b50", linewidth=1.2))
    ax.add_patch(FancyBboxPatch((0, 0), w, d, boxstyle="round,pad=0,rounding_size=3",
                                facecolor="white", edgecolor="#4c7764", linewidth=1))
    for key in board["switches"]:
        x, y = key["position"]
        kw, kd = SCREENKEY_SIZE[:2] if key["display"] else (18, 18)
        ax.add_patch(FancyBboxPatch((x - kw/2, y - kd/2), kw, kd,
                                    boxstyle="round,pad=0,rounding_size=1.3",
                                    facecolor="#c0e2e4" if key["display"] else "#eeece6",
                                    edgecolor="#758380", linewidth=0.6))
        ax.plot(x, y, "+", color="#6d8580", markersize=4)
        ax.text(x, y - 2, key["ref"], ha="center", va="top", fontsize=7)
    for m in board["mounts"]:
        x, y = m["position"]
        colour = "#c96b46" if m["ref"] in dims["post_reliefs"] else "#314d52"
        circle = Circle((x, y), 3, fill=False, edgecolor=colour)
        ax.add_patch(circle)
        if m["ref"] in dims["post_reliefs"]:
            pocket = Polygon(dims["post_reliefs"][m["ref"]]["polygon"],
                             facecolor="white", edgecolor=colour, linewidth=0.6)
            pocket.set_clip_path(circle)
            ax.add_patch(pocket)
        ax.add_patch(Circle((x, y), 1.6, fill=False, edgecolor=colour))
        ax.text(x, y + 4.3, m["ref"], ha="center", color=colour, fontsize=8)
    x = board["footprints"]["J1"]["position"][0]
    ax.add_patch(Rectangle((x - 8, d), 16, (outer_d - d) / 2 + 0.4,
                           facecolor="white", edgecolor="none"))
    ax.text(x, d - 7, "USB-C / 16 mm notch", ha="center", fontsize=8)
    ex, ey = board["footprints"]["SW17"]["position"]
    ax.add_patch(Circle((ex + 7.5, ey - 2.5), 3, fill=False, edgecolor="#69746e"))
    ax.text(ex + 7.5, ey - 9, "EC11", ha="center", fontsize=8)
    hx, hy = board["footprints"]["J2"]["position"]
    ax.text(hx + 4, hy, "OLED header", ha="center", fontsize=8)

    def dimension(a, b, label, offset=(0, 1), rotation=0):
        ax.annotate("", xy=a, xytext=b, arrowprops={"arrowstyle": "|-|", "lw": 0.7})
        ax.text((a[0]+b[0])/2 + offset[0], (a[1]+b[1])/2 + offset[1], label,
                ha="center", va="center", rotation=rotation, fontsize=9)

    dimension((-2.75, -10), (102.75, -10), "105.5 overall", (0, -3))
    dimension((-12, -2.75), (-12, 145.25), "148 overall", (-3, 0), 90)
    dimension((0, 156), (100, 156), "100 PCB", (0, 3))
    dimension((110, 0), (110, 142.5), "142.5 PCB", (3, 0), 90)
    ax.set(xlim=(-22, 122), ylim=(-20, 166), aspect="equal")
    ax.axis("off")
    fig.suptitle("KEEB 4x4  /  screw-down base", x=0.12, y=0.965, ha="left", fontsize=17)
    fig.text(0.12, 0.925, "TOP VIEW   /   dimensions in millimetres   /   PCB front-left is (0, 0)",
             fontsize=9, color="#54645f")
    fig.text(0.12, 0.073, "Top row: Waveshare caps, 22 x 25.3 mm. Other caps: 18 x 18 mm illustration.\n"
             "PCB pitch: 23 x 26.5 mm. Eight posts; H3 has a 2 mm top notch for R11.\n"
             "Base height 10.4 mm. Floor 2.4 mm. PCB underside Z = 8.4 mm.\n"
             "Scale is controlled by the dimensions; do not measure a resized printout.",
             fontsize=8, linespacing=1.6, color="#54645f")
    fig.savefig(OUT / "dimensions.svg")
    fig.savefig(OUT / "dimensions.pdf")
    plt.close(fig)


if __name__ == "__main__":
    b = read_board()
    base, _, pcb, dims = build(b)
    render(b, base, pcb, dims)
    relief_detail(b, base, dims)
    drawing(b, dims)
    print("Wrote preview.png, h3_relief_detail.png, dimensions.svg and dimensions.pdf")
