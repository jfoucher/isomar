import SwiftUI
import UIKit

/// The chart on a touch screen: the engine's tiles as Core Animation layers, an image made once per tile id
/// and then only placed, and the overlay's chunks made again only when their id changes, as on the Mac
/// (macos/Isomar/ChartView.swift), with CGImages where the Mac uses IOSurfaces. Touches become the
/// engine's drags, pinches, taps and secondary clicks (a long press).
final class TouchChartView: UIView, ChartSurface {
    private let sea = CALayer()
    private let tileRoot = CALayer()
    private let overlayRoot = CALayer()
    private var images: [UInt64: (image: CGImage, frame: Int)] = [:]
    private var chunkImages: [UInt64: CGImage] = [:]
    private var frameCount = 0
    /// A pinch pans as well as zooms (the engine keeps what was under the fingers under them), so a pan in
    /// progress stops when one begins, and starts again from where the finger is when it ends.
    private var pinching = false
    private var panning = false
    private var panRestart = false
    /// When the last pinch ended: a pan that follows at once is the finger left behind, whose speed is the
    /// pinch's and must not throw the chart.
    private var pinchEnded = Date.distantPast
    private static let sRGB = CGColorSpace(name: CGColorSpace.sRGB)!
    /// The engine's pixels: cairo's ARGB32, premultiplied, little-endian.
    private static let bgra = CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)
    /// Tile images kept after they leave the view, so that panning back makes none again.
    private static let keptImages = 120

    override init(frame: CGRect) {
        super.init(frame: frame)
        layer.masksToBounds = true
        for l in [sea, tileRoot, overlayRoot] { layer.addSublayer(l) }
        isMultipleTouchEnabled = true
        let pan = UIPanGestureRecognizer(target: self, action: #selector(pan(_:)))
        pan.maximumNumberOfTouches = 1
        let pinch = UIPinchGestureRecognizer(target: self, action: #selector(pinch(_:)))
        let double = UITapGestureRecognizer(target: self, action: #selector(doubleTap(_:)))
        double.numberOfTapsRequired = 2
        let tap = UITapGestureRecognizer(target: self, action: #selector(tap(_:)))
        tap.require(toFail: double)
        let press = UILongPressGestureRecognizer(target: self, action: #selector(press(_:)))
        for g in [pan, pinch, double, tap, press] as [UIGestureRecognizer] {
            g.delegate = self
            addGestureRecognizer(g)
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    // MARK: drawing

    private var redrawPending = false

    /// Asks for a frame on the next turn of the main loop, once however often it is asked.
    func requestRedraw() {
        guard !redrawPending else { return }
        redrawPending = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.redrawPending = false
            self.render()
        }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        isomar_resize(Double(bounds.width), Double(bounds.height))
        requestRedraw()
    }

    // MARK: ChartSurface (the layout tells the engine what covers the chart: PhoneRoot)

    func setNeedsRedraw() { requestRedraw() }

    /// A long press's actions, offered by the sheet.
    func showMenu(_ request: MenuRequest) {
        if let pick = IsomarModel.shared.autoPick, let e = request.sections.flatMap({ $0 }).first(where: { $0.label == pick }) {
            IsomarModel.shared.autoPick = nil
            isomar_menu_activate(Int32(e.i))
            return
        }
        IsomarModel.shared.menu = request
    }

    /// What was tapped, shown in the sheet.
    func showInfo(_ info: InfoRequest) { IsomarModel.shared.info = info }

    func setPointing(_ on: Bool) {}

    override func traitCollectionDidChange(_ previous: UITraitCollection?) {
        super.traitCollectionDidChange(previous)
        requestRedraw()
    }

    private func render() {
        guard bounds.width > 0, bounds.height > 0 else { return }
        let dark = traitCollection.userInterfaceStyle == .dark
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let me = Unmanaged.passUnretained(self).toOpaque()
        isomar_frame(Double(bounds.width), Double(bounds.height), Double(window?.screen.scale ?? 3), dark, me) { ctx, scene in
            guard let ctx, let scene else { return }
            Unmanaged<TouchChartView>.fromOpaque(ctx).takeUnretainedValue().apply(scene.pointee)
        }
        CATransaction.commit()
    }

    private static func colour(_ c: (Double, Double, Double, Double)) -> CGColor {
        CGColor(srgbRed: c.0, green: c.1, blue: c.2, alpha: c.3)
    }

    /// A copy of `width` x `height` pixels rows `stride` bytes apart. An overlay chunk is a window into the
    /// whole overlay, whose stride is the overlay's: its last row ends `width * 4` bytes in, and copying
    /// `stride * height` bytes read past the end of the overlay (and crashed when that crossed a page).
    private static func image(_ pixels: UnsafePointer<UInt8>, width: Int, height: Int, stride: Int) -> CGImage? {
        guard width > 0, height > 0 else { return nil }
        let row = width * 4
        let data: CFData?
        if stride == row {
            data = CFDataCreate(nil, pixels, row * height)
        } else {
            let packed = CFDataCreateMutable(nil, row * height)!
            for y in 0..<height { CFDataAppendBytes(packed, pixels + y * stride, row) }
            data = packed
        }
        guard let data, let provider = CGDataProvider(data: data) else { return nil }
        return CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: row, space: sRGB,
                       bitmapInfo: bgra, provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent)
    }

    /// Places the frame: positions are from the top-left corner, in points (UIKit's own orientation).
    private func apply(_ s: ChartScene) {
        frameCount += 1
        let scale = Double(s.scale)
        layer.backgroundColor = Self.colour(s.void_rgba)
        sea.backgroundColor = Self.colour(s.sea_rgba)
        sea.frame = CGRect(x: 0, y: s.sea_top, width: bounds.width, height: max(0, s.sea_bottom - s.sea_top))
        tileRoot.frame = bounds
        overlayRoot.frame = bounds

        var placed: [CALayer] = []
        for i in 0..<Int(s.n_tiles) {
            let t = s.tiles[i]
            var image = images[t.id]?.image
            if image == nil, let p = t.pixels {
                image = Self.image(p, width: Int(t.width), height: Int(t.height), stride: Int(t.stride))
            }
            guard let image else { continue }
            images[t.id] = (image, frameCount)
            let tile = CALayer()
            tile.contents = image
            tile.contentsGravity = .resize
            tile.magnificationFilter = .linear
            tile.minificationFilter = t.size < 192 ? .trilinear : .linear
            if t.clipped {
                let outer = CALayer()
                outer.masksToBounds = true
                outer.frame = CGRect(x: t.clip.0, y: t.clip.1, width: t.clip.2, height: t.clip.3)
                tile.frame = CGRect(x: t.x - t.clip.0, y: t.y - t.clip.1, width: t.size, height: t.size)
                outer.addSublayer(tile)
                placed.append(outer)
            } else {
                tile.frame = CGRect(x: t.x, y: t.y, width: t.size, height: t.size)
                placed.append(tile)
            }
        }
        tileRoot.sublayers = placed
        if images.count > Self.keptImages {
            for (id, _) in images.filter({ $0.value.frame != frameCount }).sorted(by: { $0.value.frame < $1.value.frame }).prefix(images.count - Self.keptImages) {
                images[id] = nil
            }
        }

        var chunks: [CALayer] = []
        var seen: [UInt64: CGImage] = [:]
        for i in 0..<Int(s.n_chunks) {
            let c = s.chunks[i]
            var image = chunkImages[c.id]
            if image == nil, let p = c.pixels {
                image = Self.image(p, width: Int(c.w), height: Int(c.h), stride: Int(c.stride))
            }
            guard let image else { continue }
            seen[c.id] = image
            let l = CALayer()
            l.contents = image
            l.contentsGravity = .resize
            l.magnificationFilter = .nearest
            l.frame = CGRect(x: Double(c.x) / scale, y: Double(c.y) / scale, width: Double(c.w) / scale, height: Double(c.h) / scale)
            chunks.append(l)
        }
        chunkImages = seen
        overlayRoot.sublayers = chunks
    }

    // MARK: touch

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        super.touchesBegan(touches, with: event)
        isomar_touch_down()
    }

    @objc private func pan(_ g: UIPanGestureRecognizer) {
        if pinching { return }
        if panRestart, g.state == .changed {
            panRestart = false
            g.setTranslation(.zero, in: self)
            let p = g.location(in: self)
            isomar_drag_begin(1, p.x, p.y)
            panning = true
            return
        }
        let t = g.translation(in: self)
        switch g.state {
        case .began:
            let p = g.location(in: self)
            isomar_drag_begin(1, p.x - t.x, p.y - t.y)
            isomar_drag_update(t.x, t.y)
            panning = true
        case .changed:
            isomar_drag_update(t.x, t.y)
        case .ended:
            if panning {
                isomar_drag_end(t.x, t.y)
                let v = g.velocity(in: self)
                if Date().timeIntervalSince(pinchEnded) > 0.3 { isomar_fling(v.x, v.y) }
            }
            panning = false
            panRestart = false
        case .cancelled, .failed:
            if panning { isomar_drag_end(t.x, t.y) }
            panning = false
            panRestart = false
        default:
            break
        }
    }

    @objc private func pinch(_ g: UIPinchGestureRecognizer) {
        let p = g.location(in: self)
        switch g.state {
        case .began:
            if panning {
                isomar_drag_end(0, 0)
                panning = false
            }
            pinching = true
            isomar_magnify_begin()
            isomar_magnify(g.scale, p.x, p.y)
        case .changed:
            // As the fingers lift one goes first, and the recogniser's location jumps from their midpoint to
            // the one left: what was under the midpoint would leap there. The zoom stays as both fingers left it.
            guard g.numberOfTouches >= 2 else { return }
            isomar_magnify(g.scale, p.x, p.y)
        default:
            pinching = false
            panRestart = true
            pinchEnded = Date()
        }
    }

    @objc private func tap(_ g: UITapGestureRecognizer) {
        let p = g.location(in: self)
        isomar_tap(1, 1, p.x, p.y)
    }

    @objc private func doubleTap(_ g: UITapGestureRecognizer) {
        let p = g.location(in: self)
        isomar_tap(1, 2, p.x, p.y)
    }

    @objc private func press(_ g: UILongPressGestureRecognizer) {
        guard g.state == .began else { return }
        let p = g.location(in: self)
        isomar_tap(3, 1, p.x, p.y)
    }
}

extension TouchChartView: UIGestureRecognizerDelegate {
    /// Pan and pinch together: a pinch takes over from a pan, and a pan resumes when it ends.
    func gestureRecognizer(_ g: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        (g is UIPinchGestureRecognizer && other is UIPanGestureRecognizer) || (g is UIPanGestureRecognizer && other is UIPinchGestureRecognizer)
    }
}

/// The chart in SwiftUI.
struct TouchChart: UIViewRepresentable {
    func makeUIView(context: Context) -> TouchChartView {
        let v = TouchChartView(frame: .zero)
        IsomarModel.shared.chartView = v
        return v
    }

    func updateUIView(_ v: TouchChartView, context: Context) {}
}
