import UIKit

/// Opaque handle for the Metal renderers `NativeVisualizerView` draws with.
/// `RidgeMetalRenderer` and `BackdropRenderer` are internal to this module —
/// building the Metal device, its command queues and the pipeline states
/// takes tens of milliseconds, so a caller outside the module builds this
/// off the main thread (the plugin's IPC queue) and hands it to the view's
/// initialiser, which only ever touches Metal objects that already exist.
public struct NativeVisualizerRenderers {
    let ridge: RidgeMetalRenderer
    let backdrop: BackdropRenderer

    /// Nil when Metal, or either renderer's shaders, are unavailable.
    public init?() {
        guard let ridge = RidgeMetalRenderer(), let backdrop = BackdropRenderer() else { return nil }
        self.ridge = ridge
        self.backdrop = backdrop
    }
}

/// The full-screen visualiser drawn natively: the album-colour backdrop
/// with the ridge over it, at full strength, the ridge running the deeper
/// full-screen stack.
///
/// The plugin places it directly above the web view while the page's
/// visualiser overlay is open. A tap (or the accessibility activate
/// action) calls `onDismiss`, which the page answers by running its own
/// close. A half-second press switches the ridge between 60 and 120
/// frames per second, remembers the choice, and shows the requested rate
/// and the measured paint rate for a few seconds.
public final class NativeVisualizerView: UIView {
    /// Called on the main thread when the viewer asks to close.
    public var onDismiss: (() -> Void)?

    let backdrop: BackdropView
    let ridge: RidgeView
    private let rateLabel = UILabel()
    private let defaults: UserDefaults
    /// Bumped on every switch, so a measurement from an earlier switch is
    /// dropped.
    private var rateGeneration = 0

    /// `renderers` is built ahead of time (`NativeVisualizerRenderers.init`,
    /// off the main thread) so this initialiser never touches Metal beyond
    /// handing existing objects to the backdrop and ridge views.
    public init(args: NativeVisualizerShowArgs, feed: NativeVisualizerFeed, renderers: NativeVisualizerRenderers, defaults: UserDefaults = .standard) {
        self.defaults = defaults
        backdrop = BackdropView(frame: .zero, tone: args.backdrop.tone, renderer: renderers.backdrop)
        ridge = RidgeView(frame: .zero, renderer: renderers.ridge)
        super.init(frame: .zero)
        backgroundColor = .black

        ridge.params = args.params
        ridge.fullScreen = true
        ridge.source = feed.reader
        ridge.frameRate = RidgeFrameRate.load(defaults)
        if let corners = args.backdrop.corners {
            backdrop.setCorners(corners, animated: false)
        }
        for child in [backdrop, ridge] as [UIView] {
            child.frame = bounds
            child.autoresizingMask = [.flexibleWidth, .flexibleHeight]
            addSubview(child)
        }

        rateLabel.font = .monospacedDigitSystemFont(ofSize: 13, weight: .medium)
        rateLabel.textColor = .white
        rateLabel.backgroundColor = UIColor.black.withAlphaComponent(0.45)
        rateLabel.layer.cornerRadius = 6
        rateLabel.layer.masksToBounds = true
        rateLabel.textAlignment = .center
        rateLabel.alpha = 0
        rateLabel.translatesAutoresizingMaskIntoConstraints = false
        addSubview(rateLabel)
        NSLayoutConstraint.activate([
            rateLabel.topAnchor.constraint(equalTo: safeAreaLayoutGuide.topAnchor, constant: 12),
            rateLabel.trailingAnchor.constraint(equalTo: safeAreaLayoutGuide.trailingAnchor, constant: -12),
            rateLabel.heightAnchor.constraint(equalToConstant: 26),
            rateLabel.widthAnchor.constraint(greaterThanOrEqualToConstant: 140),
        ])

        isAccessibilityElement = true
        accessibilityLabel = "Close visualiser"
        accessibilityTraits = .button
        // Covers the web page underneath for VoiceOver too, so swiping
        // can't navigate into content that isn't visible.
        accessibilityViewIsModal = true

        let press = UILongPressGestureRecognizer(target: self, action: #selector(longPressed(_:)))
        press.minimumPressDuration = 0.5
        let tap = UITapGestureRecognizer(target: self, action: #selector(tapped))
        tap.require(toFail: press)
        addGestureRecognizer(press)
        addGestureRecognizer(tap)
    }

    public required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    /// Start drawing.
    public func start() {
        ridge.start()
    }

    /// Stop drawing; call before removing the view.
    public func stop() {
        ridge.stop()
    }

    /// Crossfade to new corner colours. A backdrop whose corners don't have
    /// three channels each is ignored. The dim stays as it was when shown.
    public func setBackdrop(_ next: NativeBackdrop) {
        guard let corners = next.corners else { return }
        backdrop.setCorners(corners, animated: true)
    }

    public override func accessibilityActivate() -> Bool {
        handleTap()
        return true
    }

    @objc private func tapped() {
        handleTap()
    }

    @objc private func longPressed(_ gesture: UILongPressGestureRecognizer) {
        guard gesture.state == .began else { return }
        toggleFrameRate()
    }

    func handleTap() {
        onDismiss?()
    }

    /// Switch the ridge's rate, remember it, and report it: the requested
    /// rate at once, the measured paint rate after a second and a half, and
    /// the label gone about two and a half seconds after that.
    func toggleFrameRate() {
        let rate = ridge.frameRate.toggled
        ridge.frameRate = rate
        rate.save(defaults)
        rateGeneration += 1
        let generation = rateGeneration
        ridge.collectsFrameStats = true
        _ = ridge.takeFrameStats()
        showRate("\(rate.rawValue) Hz · measuring")
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { [weak self] in
            guard let self, generation == self.rateGeneration else { return }
            let stats = self.ridge.takeFrameStats()
            self.ridge.collectsFrameStats = false
            self.showRate(String(format: "%d Hz · %.0f fps", rate.rawValue, stats.paintsPerSecond))
            DispatchQueue.main.asyncAfter(deadline: .now() + 2.5) { [weak self] in
                guard let self, generation == self.rateGeneration else { return }
                UIView.animate(withDuration: 0.3) { self.rateLabel.alpha = 0 }
            }
        }
    }

    private func showRate(_ text: String) {
        rateLabel.text = "  \(text)  "
        rateLabel.alpha = 1
    }
}
