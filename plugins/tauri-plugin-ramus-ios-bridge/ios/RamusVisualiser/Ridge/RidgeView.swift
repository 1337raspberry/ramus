// Ported from ramusTV `RamusTV/Visualiser/RidgeView.swift` (Metal path only).

import QuartzCore
import UIKit

/// The ridge visualiser as a full-frame layer, driven by a display link.
///
/// The UIKit counterpart of ramus `ui/src/components/FocusVisualizer.tsx`
/// (`CanvasLayer` in ridge mode): each display frame it steps a
/// `RidgePainter` against `source` and, when the picture changed, has
/// `RidgeGeometryBuilder` turn the frame into erase spans and stroke
/// segments and `RidgeMetalRenderer` draw them into a transparent
/// `CAMetalLayer`, the history stack gliding on every display frame
/// (`glide`). Nothing is painted once every row is flat, so a pause costs no
/// GPU work. The backing store is whole device pixels and the layer's box
/// is derived from it, so the rows' pixel-grid placement survives to the
/// screen.
final class RidgeView: UIView {
    /// Where the bands come from; nil draws the line decaying to silence.
    var source: RidgeFrameSource?

    /// Milliseconds added to `ridgeSyncLeadMs` when reading the source, for
    /// output latency the audible clock doesn't see. Positive reads further
    /// ahead.
    var leadOffsetMs: Double {
        get { painter.leadOffsetMs }
        set { painter.leadOffsetMs = newValue }
    }

    /// Device pixels per point the ridge is rendered at; nil renders at the
    /// display's own scale.
    var renderScale: CGFloat? {
        didSet {
            guard renderScale != oldValue else { return }
            setNeedsLayout()
        }
    }

    /// Stroke colour of the rows (each row applies its own alpha on top).
    var lineColor: UIColor = .white {
        didSet { painter.color = Self.rgb(lineColor, traits: traitCollection) }
    }

    /// Layout, level curve, easing and sync values.
    var params: RidgeParams {
        get { painter.params }
        set {
            painter.params = newValue
            painter.invalidate()
        }
    }

    /// Draw the deeper full-screen stack (`ridgeFullScreenRows`).
    var fullScreen: Bool {
        get { painter.fullScreen }
        set {
            painter.fullScreen = newValue
            painter.invalidate()
        }
    }

    /// The display rate the link asks for.
    var frameRate: RidgeFrameRate = .sixty {
        didSet { link?.preferredFrameRateRange = frameRate.range }
    }

    /// Move the history stack on every display frame rather than once per row.
    var glide = true {
        didSet { painter.invalidate() }
    }

    /// Whether the app is in the background, checked before each frame: a
    /// background app may not submit GPU work, and with background audio a
    /// layout can still ask for a repaint there.
    var isBackgrounded: () -> Bool = { UIApplication.shared.applicationState == .background }

    /// GPU time of the latest frame, in ms.
    private(set) var lastGPUMs: Double = 0

    /// The Metal drawable's size in pixels; zero before the first layout.
    var drawablePixelSize: CGSize { metalLayer.drawableSize }

    /// True between `start()` and `stop()`. The display link only runs while
    /// the view is in a window as well.
    private(set) var isRunning = false

    /// Frames painted since the view was created (never reset), and the time
    /// the latest paint took (step, geometry and hand-off to the GPU), in ms.
    private(set) var paintsCompleted = 0
    private(set) var lastPaintMs: Double = 0

    /// Collect frame timing for `takeFrameStats`; off by default, as the
    /// samples pile up until they are taken.
    var collectsFrameStats = false

    /// Frame timing since the last call.
    func takeFrameStats() -> RidgeFrameStats {
        var taken = stats
        taken.seconds = statsStart.map { CACurrentMediaTime() - $0 } ?? 0
        stats = RidgeFrameStats()
        statsStart = CACurrentMediaTime()
        return taken
    }

    private var stats = RidgeFrameStats()
    private var statsStart: CFTimeInterval?
    private var lastLinkTimestamp: CFTimeInterval?
    private var lastCut: (rows: Int, at: CFTimeInterval)?

    private let painter = RidgePainter()
    private let metal: RidgeMetalRenderer
    private let metalLayer = CAMetalLayer()
    private var metalTarget: RidgeTarget?
    private let geometry = RidgeGeometryBuilder()
    private var drawList = RidgeDrawList()
    private var link: CADisplayLink?

    /// Relays display-link ticks without the link retaining the view.
    private final class LinkTarget: NSObject {
        weak var view: RidgeView?

        @objc func tick(_ link: CADisplayLink) {
            view?.tick(link)
        }
    }

    init(frame: CGRect, renderer: RidgeMetalRenderer) {
        metal = renderer
        super.init(frame: frame)
        backgroundColor = .clear
        isOpaque = false
        isUserInteractionEnabled = false
        metalLayer.device = renderer.device
        metalLayer.pixelFormat = RidgeMetalRenderer.pixelFormat
        metalLayer.framebufferOnly = true
        metalLayer.isOpaque = false
        metalLayer.colorspace = CGColorSpace(name: CGColorSpace.sRGB)
        metalLayer.maximumDrawableCount = RidgeMetalRenderer.framesInFlight
        metalLayer.presentsWithTransaction = false
        layer.addSublayer(metalLayer)
        renderer.onGPUTime = { [weak self] ms in
            guard let self else { return }
            self.lastGPUMs = ms
            if self.collectsFrameStats { self.stats.gpuMs.append(ms) }
        }
        registerForTraitChanges([UITraitDisplayScale.self, UITraitUserInterfaceStyle.self]) {
            (view: RidgeView, _: UITraitCollection) in
            view.painter.color = RidgeView.rgb(view.lineColor, traits: view.traitCollection)
            view.setNeedsLayout()
        }
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    deinit {
        link?.invalidate()
    }

    /// Start painting on every display frame while the view is in a window.
    func start() {
        guard !isRunning else { return }
        isRunning = true
        setNeedsLayout()
        let target = LinkTarget()
        target.view = self
        let link = CADisplayLink(target: target, selector: #selector(LinkTarget.tick(_:)))
        link.preferredFrameRateRange = frameRate.range
        link.isPaused = window == nil
        link.add(to: .main, forMode: .common)
        self.link = link
        Log.visualiser.debug("ridge started")
    }

    /// Stop painting; the layer keeps its last frame.
    func stop() {
        guard isRunning else { return }
        isRunning = false
        link?.invalidate()
        link = nil
        Log.visualiser.debug("ridge stopped")
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        link?.isPaused = window == nil
        if window != nil { setNeedsLayout() }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let native = traitCollection.displayScale > 0 ? traitCollection.displayScale : 1
        let scale = max(0.25, renderScale ?? native)
        // Whole device pixels, with the layer's box derived from them.
        let pw = Int((bounds.width * scale).rounded(.down))
        let ph = Int((bounds.height * scale).rounded(.down))
        guard isRunning, pw > 0, ph > 0 else {
            metalTarget = nil
            return
        }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        metalLayer.frame = CGRect(x: 0, y: 0, width: CGFloat(pw) / scale, height: CGFloat(ph) / scale)
        metalLayer.contentsScale = scale
        CATransaction.commit()
        let target = RidgeTarget(pixelWidth: pw, pixelHeight: ph, scale: Double(scale))
        guard target != metalTarget else { return }
        metalLayer.drawableSize = CGSize(width: pw, height: ph)
        metalTarget = target
        painter.invalidate()
        Log.visualiser.debug("ridge drawable \(pw)x\(ph) at \(Double(scale))x")
        // Repaint straight away rather than leave the stretched old frame up
        // until the next tick.
        tick(nil)
    }

    private func tick(_ link: CADisplayLink?) {
        if let link {
            let period = link.targetTimestamp - link.timestamp
            if period > 0 { painter.framePeriodMs = period * 1000 }
            if collectsFrameStats { noteTick(link) }
        }
        let now = CACurrentMediaTime() * 1000
        guard let target = metalTarget else { return }
        guard !isBackgrounded() else {
            // Repaint on the first frame back in the foreground.
            painter.invalidate()
            return
        }
        guard painter.step(
            now: now, frameTime: link.map { $0.targetTimestamp * 1000 }, source: source,
            height: target.height)
        else { return }
        geometry.build(painter.frame(), target: target, glide: glide, into: &drawList)
        guard metal.render(drawList, to: metalLayer) else {
            // No drawable, or the GPU is behind: repaint on the next tick.
            painter.invalidate()
            return
        }
        notePaint(startedAt: now, link: link)
    }

    private func notePaint(startedAt now: Double, link: CADisplayLink?) {
        paintsCompleted += 1
        lastPaintMs = CACurrentMediaTime() * 1000 - now
        guard collectsFrameStats else { return }
        stats.paintMs.append(lastPaintMs)
        if let link {
            if let cut = lastCut, painter.rowsCut != cut.rows {
                let period = link.targetTimestamp - link.timestamp
                let gap = period > 0 ? Int(((link.timestamp - cut.at) / period).rounded()) : 0
                stats.cutGaps[min(3, max(1, gap)) - 1] += 1
            }
            if lastCut?.rows != painter.rowsCut { lastCut = (painter.rowsCut, link.timestamp) }
        }
    }

    private func noteTick(_ link: CADisplayLink) {
        let period = link.targetTimestamp - link.timestamp
        stats.ticks += 1
        stats.periodMs = period * 1000
        stats.lateMs.append((CACurrentMediaTime() - link.timestamp) * 1000)
        if let last = lastLinkTimestamp, period > 0 {
            stats.missed += max(0, Int(((link.timestamp - last) / period).rounded()) - 1)
        }
        lastLinkTimestamp = link.timestamp
        if statsStart == nil { statsStart = CACurrentMediaTime() }
    }

    private static func rgb(_ color: UIColor, traits: UITraitCollection) -> RidgeRGB {
        var r: CGFloat = 1, g: CGFloat = 1, b: CGFloat = 1, a: CGFloat = 1
        guard color.resolvedColor(with: traits).getRed(&r, green: &g, blue: &b, alpha: &a) else {
            return .white
        }
        return RidgeRGB(red: r, green: g, blue: b)
    }
}
