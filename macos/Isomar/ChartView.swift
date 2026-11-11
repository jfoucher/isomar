import AppKit
import IOSurface
import SwiftUI

/// The chart. The engine's tiles become Core Animation layers, each an image made once and then only
/// placed, and the layers it draws every frame come as chunks of an overlay, of which only those that
/// changed are made again: a pan moves layers on the GPU instead of copying every pixel of the view into
/// a new bitmap each frame. `ISOMAR_CAIRO_CANVAS=1` draws the whole view into one bitmap, as before.
final class ChartNSView: NSView, ChartSurface {
    private var bitmap: CGContext?
    private var pressAt = NSPoint.zero
    private var dragging = false
    private var pinch = 1.0
    private var pinching = false
    private var panning = false
    private var panDelta = NSPoint.zero
    private var panEnd: Timer?

    private let cairoCanvas = ProcessInfo.processInfo.environment["ISOMAR_CAIRO_CANVAS"] != nil
    private let sea = CALayer()
    private let tileRoot = CALayer()
    private let overlayRoot = CALayer()
    /// Tile images by tile id, with the frame that last showed them and their size in bytes. IOSurfaces,
    /// which Core Animation takes as they are: a CGImage it copied by drawing it, on the main thread.
    private var images: [UInt64: (image: IOSurface, frame: Int, bytes: Int)] = [:]
    /// The layers placing them, by tile and the rectangle it is cut to.
    private var tileLayers: [TileSpot: CALayer] = [:]
    /// Overlay chunks by their corner.
    private var chunkLayers: [ChunkSpot: ChunkLayer] = [:]
    private var frameCount = 0
    /// Bytes of tile images kept after they leave the view, so that panning back makes none again.
    private static let imageBudget = 160 << 20
    private static let sRGB = CGColorSpace(name: CGColorSpace.sRGB)!
    /// The engine's pixels: cairo's ARGB32, premultiplied, little-endian.
    private static let bgra = CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)

    private struct TileSpot: Hashable {
        let id: UInt64
        let clip: [Double]
    }

    private struct ChunkSpot: Hashable {
        let x: Int32
        let y: Int32
    }

    /// A chunk of the overlay on screen. Its pixels change at every frame of a pan: they are written
    /// into one of a few surfaces kept for it, in turn, rather than into a new one each time.
    private final class ChunkLayer {
        var id: UInt64 = 0
        let layer = CALayer()
        var surfaces: [IOSurface] = []
        var next = 0

        init() {
            layer.contentsGravity = .resize
            layer.magnificationFilter = .nearest
        }

        func show(_ c: ChartChunk) {
            guard let src = c.pixels else { return }
            let (w, h) = (Int(c.w), Int(c.h))
            // One Core Animation is not still reading.
            var s = surfaces.indices.lazy.map { (self.next + $0) % self.surfaces.count }.first { self.surfaces[$0].width == w && self.surfaces[$0].height == h && !self.surfaces[$0].isInUse }
            if s == nil, let made = ChartNSView.newSurface(width: w, height: h) {
                if surfaces.count >= 3 { surfaces[next % 3] = made; s = next % 3 } else { surfaces.append(made); s = surfaces.count - 1 }
            }
            guard let s else { return }
            ChartNSView.write(surfaces[s], src, width: w, height: h, stride: Int(c.stride))
            layer.contents = surfaces[s]
            next = s + 1
            id = c.id
        }
    }

    override var wantsUpdateLayer: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override var isFlipped: Bool { false }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.contentsGravity = .topLeft
        layer?.backgroundColor = NSColor.black.cgColor
        if !cairoCanvas {
            // Tiles at the edges reach past the view.
            layer?.masksToBounds = true
            for l in [sea, tileRoot, overlayRoot] { layer?.addSublayer(l) }
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    /// Space plays and pauses the time control, wherever the keys are going in the window, except into
    /// text: after Play in the sidebar the chart does not have them.
    private var spaceMonitor: Any?

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if let m = spaceMonitor { NSEvent.removeMonitor(m); spaceMonitor = nil }
        guard window != nil else { return }
        spaceMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] e in
            guard e.keyCode == 49, e.modifierFlags.intersection([.command, .option, .control, .shift]).isEmpty,
                  let w = self?.window, e.window === w, w.attachedSheet == nil, !(w.firstResponder is NSText),
                  let t = IsomarModel.shared.timeline else { return e }
            IsomarModel.shared.send("timeline_play", ["on": !t.playing])
            return nil
        }
    }

    func setNeedsRedraw() { needsDisplay = true }

    private var isDark: Bool {
        effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
    }

    override func updateLayer() {
        let scale = window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2
        if cairoCanvas {
            let w = max(1, Int((bounds.width * scale).rounded())), h = max(1, Int((bounds.height * scale).rounded()))
            if bitmap == nil || bitmap!.width != w || bitmap!.height != h {
                bitmap = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0, space: Self.sRGB, bitmapInfo: Self.bgra.rawValue)
            }
            guard let ctx = bitmap, let data = ctx.data else { return }
            isomar_render(data.assumingMemoryBound(to: UInt8.self), Int32(w), Int32(h), Int32(ctx.bytesPerRow), Double(scale), isDark)
            layer?.contentsScale = scale
            layer?.contents = ctx.makeImage()
            return
        }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let me = Unmanaged.passUnretained(self).toOpaque()
        isomar_frame(Double(bounds.width), Double(bounds.height), Double(scale), isDark, me) { ctx, scene in
            guard let ctx, let scene else { return }
            Unmanaged<ChartNSView>.fromOpaque(ctx).takeUnretainedValue().apply(scene.pointee)
        }
        CATransaction.commit()
    }

    private static func colour(_ c: (Double, Double, Double, Double)) -> CGColor {
        CGColor(srgbRed: c.0, green: c.1, blue: c.2, alpha: c.3)
    }

    /// Places the frame's tiles and chunks. Positions come from the top-left corner, in points.
    private func apply(_ s: ChartScene) {
        frameCount += 1
        let (w, h) = (Double(bounds.width), Double(bounds.height))
        let scale = Double(s.scale)
        func rect(_ x: Double, _ y: Double, _ rw: Double, _ rh: Double, in height: Double) -> CGRect {
            CGRect(x: x, y: height - y - rh, width: rw, height: rh)
        }
        layer?.backgroundColor = Self.colour(s.void_rgba)
        sea.backgroundColor = Self.colour(s.sea_rgba)
        sea.frame = rect(0, s.sea_top, w, max(0, s.sea_bottom - s.sea_top), in: h)
        tileRoot.frame = bounds
        overlayRoot.frame = bounds

        var placed: [CALayer] = []
        var spots: [TileSpot: CALayer] = [:]
        for i in 0..<Int(s.n_tiles) {
            let t = s.tiles[i]
            guard let image = image(for: t) else { continue }
            let clip = t.clipped ? [t.clip.0, t.clip.1, t.clip.2, t.clip.3] : []
            let spot = TileSpot(id: t.id, clip: clip)
            let exact = !t.clipped && abs(t.size - 256) < 1e-9
            let outer = spots[spot] ?? tileLayers[spot] ?? Self.tileLayer(image: image, clipped: t.clipped)
            let tile = t.clipped ? outer.sublayers![0] : outer
            tile.magnificationFilter = exact ? .nearest : .linear
            tile.minificationFilter = t.size < 192 ? .trilinear : .linear
            if t.clipped {
                // Cut to the missing tile it stands in for: a layer that clips, with the tile inside it.
                outer.frame = rect(t.clip.0, t.clip.1, t.clip.2, t.clip.3, in: h)
                tile.frame = rect(t.x - t.clip.0, t.y - t.clip.1, t.size, t.size, in: t.clip.3)
            } else {
                outer.frame = rect(t.x, t.y, t.size, t.size, in: h)
            }
            spots[spot] = outer
            placed.append(outer)
        }
        tileLayers = spots
        tileRoot.sublayers = placed
        forgetImages()

        var chunks: [ChunkSpot: ChunkLayer] = [:]
        for i in 0..<Int(s.n_chunks) {
            let c = s.chunks[i]
            let spot = ChunkSpot(x: c.x, y: c.y)
            let entry = chunkLayers[spot] ?? ChunkLayer()
            if entry.id != c.id { entry.show(c) }
            entry.layer.frame = rect(Double(c.x) / scale, Double(c.y) / scale, Double(c.w) / scale, Double(c.h) / scale, in: h)
            chunks[spot] = entry
        }
        chunkLayers = chunks
        overlayRoot.sublayers = chunks.values.map(\.layer)
    }

    private static func tileLayer(image: IOSurface, clipped: Bool) -> CALayer {
        let l = CALayer()
        l.contents = image
        l.contentsGravity = .resize
        l.isOpaque = false
        guard clipped else { return l }
        let outer = CALayer()
        outer.masksToBounds = true
        outer.addSublayer(l)
        return outer
    }

    /// The tile's image, made from its pixels the first time it shows.
    private func image(for t: ChartTile) -> IOSurface? {
        if let e = images[t.id] {
            images[t.id]?.frame = frameCount
            return e.image
        }
        guard let pixels = t.pixels, let image = Self.surface(pixels, width: Int(t.width), height: Int(t.height), stride: Int(t.stride)) else { return nil }
        images[t.id] = (image, frameCount, image.allocationSize)
        return image
    }

    /// Pixels (BGRA premultiplied, `stride` bytes to a row) copied into a new IOSurface.
    private static func surface(_ pixels: UnsafePointer<UInt8>, width: Int, height: Int, stride: Int) -> IOSurface? {
        guard let s = newSurface(width: width, height: height) else { return nil }
        write(s, pixels, width: width, height: height, stride: stride)
        return s
    }

    fileprivate static func newSurface(width: Int, height: Int) -> IOSurface? {
        IOSurface(properties: [.width: width, .height: height, .bytesPerElement: 4, .pixelFormat: 0x4247_5241 /* 'BGRA' */])
    }

    fileprivate static func write(_ s: IOSurface, _ pixels: UnsafePointer<UInt8>, width: Int, height: Int, stride: Int) {
        s.lock(options: [], seed: nil)
        let (dst, row, rowBytes) = (s.baseAddress, s.bytesPerRow, width * 4)
        if row == stride {
            dst.copyMemory(from: pixels, byteCount: stride * (height - 1) + rowBytes)
        } else {
            var y = 0
            while y < height {
                (dst + y * row).copyMemory(from: pixels + y * stride, byteCount: rowBytes)
                y += 1
            }
        }
        s.unlock(options: [], seed: nil)
    }

    /// Drops the images of tiles out of view, the longest unseen first, past the budget.
    private func forgetImages() {
        var held = images.values.reduce(0) { $0 + $1.bytes }
        guard held > Self.imageBudget else { return }
        for (id, e) in images.filter({ $0.value.frame != frameCount }).sorted(by: { $0.value.frame < $1.value.frame }) {
            if held <= Self.imageBudget { break }
            images[id] = nil
            held -= e.bytes
        }
    }

    // MARK: size and appearance

    private var laidOut = CGSize.zero

    override func layout() {
        super.layout()
        // Only a new size: telling the engine moves the view, and the footer it updates lays the window out again.
        guard bounds.size != laidOut else { return }
        laidOut = bounds.size
        isomar_resize(Double(bounds.width), Double(bounds.height))
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        needsDisplay = true
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        needsDisplay = true
    }

    // MARK: pointer

    private func point(_ event: NSEvent) -> NSPoint {
        let p = convert(event.locationInWindow, from: nil)
        return NSPoint(x: p.x, y: bounds.height - p.y)
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        for a in trackingAreas { removeTrackingArea(a) }
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self))
    }

    override func mouseMoved(with event: NSEvent) {
        let p = point(event)
        isomar_pointer(p.x, p.y, true)
    }

    override func mouseExited(with event: NSEvent) { isomar_pointer(0, 0, false) }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        pressAt = point(event)
        dragging = false
        isomar_drag_begin(1, pressAt.x, pressAt.y)
    }

    override func mouseDragged(with event: NSEvent) {
        let p = point(event)
        let dx = p.x - pressAt.x, dy = p.y - pressAt.y
        if abs(dx) > 3 || abs(dy) > 3 { dragging = true }
        isomar_drag_update(dx, dy)
        isomar_pointer(p.x, p.y, true)
    }

    override func mouseUp(with event: NSEvent) {
        let p = point(event)
        isomar_drag_end(p.x - pressAt.x, p.y - pressAt.y)
        if !dragging {
            // Control-click is a secondary click on a Mac.
            let button: UInt32 = event.modifierFlags.contains(.control) ? 3 : 1
            isomar_click(button, Int32(event.clickCount), p.x, p.y)
        }
    }

    override func rightMouseDown(with event: NSEvent) { window?.makeFirstResponder(self) }

    override func rightMouseUp(with event: NSEvent) {
        let p = point(event)
        isomar_click(3, 1, p.x, p.y)
    }

    override func otherMouseDown(with event: NSEvent) {
        pressAt = point(event)
        isomar_drag_begin(2, pressAt.x, pressAt.y)
    }

    override func otherMouseDragged(with event: NSEvent) {
        let p = point(event)
        isomar_drag_update(p.x - pressAt.x, p.y - pressAt.y)
    }

    override func otherMouseUp(with event: NSEvent) {
        let p = point(event)
        isomar_drag_end(p.x - pressAt.x, p.y - pressAt.y)
    }

    // MARK: scroll and pinch

    override func scrollWheel(with event: NSEvent) {
        let p = point(event)
        if !event.hasPreciseScrollingDeltas || event.modifierFlags.contains(.option) {
            // A wheel notch, or ⌥ with a trackpad, zooms about the pointer.
            let dy = event.hasPreciseScrollingDeltas ? event.scrollingDeltaY / 30 : event.scrollingDeltaY
            isomar_scroll_zoom(-dy, p.x, p.y)
            return
        }
        // A trackpad pans, as a drag of the middle button would.
        if !panning {
            panning = true
            panDelta = .zero
            isomar_drag_begin(2, p.x, p.y)
        }
        panDelta.x += event.scrollingDeltaX
        panDelta.y += event.scrollingDeltaY
        isomar_drag_update(panDelta.x, panDelta.y)
        panEnd?.invalidate()
        panEnd = Timer.scheduledTimer(withTimeInterval: 0.15, repeats: false) { [weak self] _ in
            guard let self, self.panning else { return }
            self.panning = false
            isomar_drag_end(self.panDelta.x, self.panDelta.y)
        }
    }

    override func magnify(with event: NSEvent) {
        let p = point(event)
        // A pinch whose start was not seen (it began over a popover, or during a pan) starts here, rather
        // than going on from the last pinch's scale and anchor.
        if event.phase.contains(.began) || !pinching {
            pinch = 1
            pinching = true
            isomar_magnify_begin()
        }
        // One step of a fast pinch out can say -1 or less: the scale would reach nothing.
        pinch *= max(0.1, 1 + Double(event.magnification))
        isomar_magnify(pinch, p.x, p.y)
        if event.phase.contains(.ended) || event.phase.contains(.cancelled) { pinching = false }
    }

    // MARK: menus and popovers

    func showMenu(_ request: MenuRequest) {
        // At night the menu is a popover over a solid fill: NSMenu would be a light, material panel.
        if Theme.isNight && IsomarModel.shared.autoPick == nil {
            menuPopover?.close()
            let p = NSPopover()
            p.behavior = .transient
            p.appearance = NSAppearance(named: .darkAqua)
            p.contentViewController = NSHostingController(rootView: NightMenu(request: request) { [weak self] i in
                self?.menuPopover?.close()
                isomar_menu_activate(Int32(i))
            })
            p.show(relativeTo: NSRect(x: request.x, y: bounds.height - request.y, width: 1, height: 1), of: self, preferredEdge: .maxX)
            menuPopover = p
            return
        }
        if let label = IsomarModel.shared.autoPick {
            IsomarModel.shared.autoPick = nil
            if let e = request.sections.flatMap({ $0 }).first(where: { $0.label == label }) {
                isomar_menu_activate(Int32(e.i))
            } else {
                FileHandle.standardError.write(Data("isomar: no menu entry \(label)\n".utf8))
            }
            return
        }
        let menu = NSMenu()
        for (n, section) in request.sections.enumerated() {
            if n > 0 { menu.addItem(.separator()) }
            for e in section {
                let item = NSMenuItem(title: e.label, action: #selector(menuPicked(_:)), keyEquivalent: "")
                item.tag = e.i
                item.target = self
                menu.addItem(item)
            }
        }
        menu.popUp(positioning: nil, at: NSPoint(x: request.x, y: bounds.height - request.y), in: self)
    }

    @objc private func menuPicked(_ item: NSMenuItem) {
        isomar_menu_activate(Int32(item.tag))
    }

    private var popover: NSPopover?
    private var menuPopover: NSPopover?

    func showInfo(_ info: InfoRequest) {
        // Stepping to another feature under the click changes what the open popover says.
        if let p = popover, p.isShown, info.choice != nil, let host = p.contentViewController as? NSHostingController<InfoView> {
            host.rootView = InfoView(info: info)
            return
        }
        // Not through the delegate: this popover is being replaced, not dismissed.
        let old = popover
        popover = nil
        old?.close()
        let p = NSPopover()
        p.behavior = .transient
        p.delegate = self
        let host = NSHostingController(rootView: InfoView(info: info))
        host.sizingOptions = .preferredContentSize
        p.contentViewController = host
        p.show(relativeTo: NSRect(x: info.x, y: bounds.height - info.y, width: 1, height: 1), of: self, preferredEdge: .maxY)
        popover = p
    }

    private var pointing = false

    /// The pointer is over something a click opens: a hand.
    func setPointing(_ on: Bool) {
        guard on != pointing else { return }
        pointing = on
        window?.invalidateCursorRects(for: self)
    }

    override func resetCursorRects() {
        if pointing { addCursorRect(bounds, cursor: .pointingHand) }
    }

    // MARK: keys

    override func keyDown(with event: NSEvent) {
        let key: Int
        switch event.keyCode {
        case 123: key = ISOMAR_KEY_LEFT
        case 124: key = ISOMAR_KEY_RIGHT
        case 125: key = ISOMAR_KEY_DOWN
        case 126: key = ISOMAR_KEY_UP
        case 53: key = ISOMAR_KEY_ESCAPE
        case 36, 76: key = ISOMAR_KEY_ENTER
        case 51: key = ISOMAR_KEY_BACKSPACE
        default:
            switch event.charactersIgnoringModifiers {
            case "+", "=": key = ISOMAR_KEY_ZOOM_IN
            case "-": key = ISOMAR_KEY_ZOOM_OUT
            case "f": IsomarModel.shared.cycleFollow(); return
            case "m": IsomarModel.shared.toggleMeasure(); return
            default: key = ISOMAR_KEY_OTHER
            }
        }
        if !isomar_key(Int32(key)) { super.keyDown(with: event) }
    }
}

extension ChartNSView: NSPopoverDelegate {
    func popoverDidClose(_ notification: Notification) {
        if (notification.object as? NSPopover) === popover {
            popover = nil
            IsomarModel.shared.command("info_closed")
        }
    }
}

struct ChartRepresentable: NSViewRepresentable {
    func makeNSView(context: Context) -> ChartNSView {
        let v = ChartNSView(frame: .zero)
        IsomarModel.shared.chartView = v
        return v
    }

    func updateNSView(_ nsView: ChartNSView, context: Context) {}
}

/// What a mark on the chart is, in a popover.
struct InfoView: View {
    let info: InfoRequest

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let c = info.choice, c.count == 2 {
                HStack {
                    Button { IsomarModel.shared.command("info_cycle", -1) } label: { Image(systemName: "chevron.left") }
                    Text("\(c[0]) of \(c[1])").foregroundStyle(.secondary).frame(maxWidth: .infinity)
                    Button { IsomarModel.shared.command("info_cycle", 1) } label: { Image(systemName: "chevron.right") }
                }
                .buttonStyle(.borderless)
            }
            Text(info.title).font(.headline)
            if let s = info.subtitle { Text(s).foregroundStyle(.secondary) }
            Grid(alignment: .topLeading, horizontalSpacing: 12, verticalSpacing: 2) {
                ForEach(Array(info.rows.enumerated()), id: \.offset) { _, row in
                    GridRow {
                        Text(row[0]).foregroundStyle(.secondary)
                        Text(row[1]).textSelection(.enabled)
                    }
                }
            }
            .padding(.top, 4)
        }
        .padding(12)
        .frame(minWidth: 280, maxWidth: 480, alignment: .leading)
    }
}

/// The right-click menu at night: a list in a solid popover.
struct NightMenu: View {
    let request: MenuRequest
    let pick: (Int) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(request.sections.enumerated()), id: \.offset) { n, section in
                if n > 0 { Divider().padding(.vertical, 3) }
                ForEach(section, id: \.i) { e in
                    Button { pick(e.i) } label: { Text(e.label).frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 4).padding(.horizontal, 10).contentShape(Rectangle()) }
                        .buttonStyle(.plain)
                }
            }
        }
        .padding(6).frame(minWidth: 220)
        .background(Color.isomar(.popoverBg))
        .foregroundStyle(Color.isomar(.text))
    }
}
