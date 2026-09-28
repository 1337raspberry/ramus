// Ported from ramusTV `RamusTV/Backdrop/BackdropView.swift` (colours are
// given rather than extracted from art).

import Metal
import QuartzCore
import UIKit

/// The album-colour backdrop behind the ridge: ramus's four-corner field
/// (`BackdropField`), drawn by `BackdropRenderer` into an opaque
/// `CAMetalLayer`. The layer is sized in whole device pixels at the
/// display's own scale, so each layer pixel is one screen pixel and the
/// dither reaches the screen as drawn. For the same reason the layer is
/// never faded with `opacity`: a switch to black fades the shader's
/// strength instead.
///
/// It draws only when the picture changes:
/// - new colours or strength (a display link runs while they fade);
/// - a new size or scale;
/// - a return to the foreground after a draw was skipped there, as a
///   background app may submit no GPU work.
///
/// A draw that finds the previous frame still on the GPU is skipped and
/// made on the next display frame, so the main thread never waits on the
/// GPU or the display.
final class BackdropView: UIView {
    let tone: BackdropTone

    /// The corners shown or being faded to, before the tone pass.
    private(set) var targetCorners: BackdropCorners?

    /// True when the field is shown, false when faded (or fading) to black.
    private(set) var showsColors = true

    /// True while a colour or strength fade is running.
    var isAnimating: Bool { animator.isAnimating }

    /// False once a fade to black has finished.
    var isFieldVisible: Bool { !metalLayer.isHidden }

    /// The drawable's size in pixels; zero before the first layout.
    var drawablePixelSize: CGSize { metalLayer.drawableSize }

    /// The layer's drawable count.
    var maximumDrawableCount: Int { metalLayer.maximumDrawableCount }

    /// Frames drawn since the view was created.
    private(set) var drawsCompleted = 0

    /// True while the latest picture is still to be drawn: its draw was
    /// skipped because the previous frame was on the GPU or the app was in
    /// the background.
    var isDrawPending: Bool { needsDraw }

    /// The clock fades run on, in seconds.
    var now: () -> Double = { CACurrentMediaTime() }

    /// Checked before each draw: a background app may submit no GPU work.
    var isBackgrounded: () -> Bool = { UIApplication.shared.applicationState == .background }

    private let metalLayer = CAMetalLayer()
    private let renderer: BackdropRenderer
    private var animator = BackdropAnimator()
    /// The display link driving a running fade; read-only outside the type
    /// so tests can check its rate.
    private(set) var link: CADisplayLink?
    private var needsDraw = false
    private var activeObserver: NSObjectProtocol?

    /// Relays display-link ticks without the link retaining the view.
    private final class LinkTarget: NSObject {
        weak var view: BackdropView?

        @objc func tick(_ link: CADisplayLink) {
            view?.tick()
        }
    }

    init(frame: CGRect = .zero, tone: BackdropTone, renderer: BackdropRenderer) {
        self.tone = tone
        self.renderer = renderer
        super.init(frame: frame)
        isUserInteractionEnabled = false
        backgroundColor = .clear
        metalLayer.device = renderer.device
        metalLayer.pixelFormat = BackdropRenderer.pixelFormat
        metalLayer.framebufferOnly = true
        // A fade draws on every display frame. With two drawables each draw
        // waited for the display to release one, about two frames; a third
        // is free by then.
        metalLayer.maximumDrawableCount = 3
        metalLayer.isOpaque = true
        metalLayer.colorspace = CGColorSpace(name: CGColorSpace.sRGB)
        layer.addSublayer(metalLayer)
        registerForTraitChanges([UITraitDisplayScale.self]) { (view: BackdropView, _: UITraitCollection) in
            view.setNeedsLayout()
        }
        activeObserver = NotificationCenter.default.addObserver(
            forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            guard let self, self.needsDraw else { return }
            self.draw()
        }
        setCorners(.brandDefault, animated: false)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    deinit {
        link?.invalidate()
        if let activeObserver { NotificationCenter.default.removeObserver(activeObserver) }
    }

    /// Shows `corners` (before the tone pass), crossfading when `animated`
    /// and the field is visible.
    func setCorners(_ corners: BackdropCorners, animated: Bool) {
        targetCorners = corners
        let visible = animator.strength > 0 || animator.targetStrength > 0
        let colors = BackdropFieldColors(corners, tone: tone)
        guard animator.setColors(colors, now: now(), animated: animated && visible) else { return }
        pictureChanged()
    }

    /// Shows the field, or fades it to black and hides it.
    func setShowsColors(_ shows: Bool, animated: Bool) {
        showsColors = shows
        guard animator.setStrength(shows ? 1 : 0, now: now(), animated: animated) else { return }
        pictureChanged()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let scale = traitCollection.displayScale > 0 ? traitCollection.displayScale : 1
        // Whole device pixels, with the layer's box derived from them.
        let pw = (bounds.width * scale).rounded(.down)
        let ph = (bounds.height * scale).rounded(.down)
        guard pw > 0, ph > 0 else { return }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        metalLayer.frame = CGRect(x: 0, y: 0, width: pw / scale, height: ph / scale)
        metalLayer.contentsScale = scale
        CATransaction.commit()
        let size = CGSize(width: pw, height: ph)
        guard metalLayer.drawableSize != size else { return }
        metalLayer.drawableSize = size
        draw()
    }

    /// Draws the change, and keeps drawing on each display frame while a
    /// fade runs.
    private func pictureChanged() {
        updateVisibility()
        draw()
        if animator.isAnimating { startLink() }
    }

    private func startLink() {
        guard link == nil else { return }
        let target = LinkTarget()
        target.view = self
        let link = CADisplayLink(target: target, selector: #selector(LinkTarget.tick(_:)))
        // The app disables the minimum-frame-duration throttle so the ridge
        // can run at 120 Hz; without a range here this crossfade would tick
        // at whatever the display's top rate is too.
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 60, preferred: 60)
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    private func tick() {
        let running = animator.step(now: now())
        updateVisibility()
        draw()
        // A skipped draw is made on the next frame; one skipped in the
        // background waits for the app to become active.
        if !running, !needsDraw || isBackgrounded() {
            link?.invalidate()
            link = nil
        }
    }

    /// Hidden once the field has faded to black and is staying there.
    private func updateVisibility() {
        let hidden = animator.strength == 0 && animator.targetStrength == 0
        guard metalLayer.isHidden != hidden else { return }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        metalLayer.isHidden = hidden
        CATransaction.commit()
    }

    private func draw() {
        guard !metalLayer.isHidden, metalLayer.drawableSize.width > 0, let colors = animator.shown else {
            // Nothing to draw until the field shows or is laid out, which
            // draws anyway.
            needsDraw = false
            return
        }
        guard !isBackgrounded() else {
            needsDraw = true
            return
        }
        let uniforms = BackdropUniforms(colors: colors, tone: tone, strength: animator.strength)
        if renderer.render(uniforms, to: metalLayer) {
            drawsCompleted += 1
            needsDraw = false
        } else {
            needsDraw = true
            startLink()
        }
    }
}
