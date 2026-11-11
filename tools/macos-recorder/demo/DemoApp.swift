// Spike, throwaway: a tiny AppKit window to record. A label counts up every half second and
// a bar slides, so a film has motion and a still has text. DEMO_COLOR=teal changes the look,
// DEMO_X moves the window (offscreen when negative), DEMO_MINIMIZE minimizes it,
// DEMO_STATIC keeps it still.

import AppKit

final class Delegate: NSObject, NSApplicationDelegate {
    var window: NSWindow!
    var label: NSTextField!
    var bar: NSView!
    var count = 0

    func applicationDidFinishLaunching(_ notification: Notification) {
        let env = ProcessInfo.processInfo.environment
        let teal = env["DEMO_COLOR"] == "teal"
        let text = env["DEMO_TEXT"] ?? "Count 0"
        let x = Double(env["DEMO_X"] ?? "") ?? 200
        window = NSWindow(
            contentRect: NSRect(x: x, y: 200, width: 480, height: 320),
            styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.title = "Look Demo"
        let root = NSView(frame: window.contentView!.bounds)
        root.wantsLayer = true
        root.layer?.backgroundColor = (teal ? NSColor.systemTeal : NSColor.systemPurple).cgColor
        label = NSTextField(labelWithString: text)
        label.font = .systemFont(ofSize: 48, weight: .bold)
        label.textColor = .white
        label.frame = NSRect(x: 40, y: 180, width: 400, height: 70)
        root.addSubview(label)
        bar = NSView(frame: NSRect(x: 0, y: 60, width: 80, height: 30))
        bar.wantsLayer = true
        bar.layer?.backgroundColor = NSColor.white.cgColor
        root.addSubview(bar)
        window.contentView = root
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        if env["DEMO_X"] != nil { window.setFrameOrigin(NSPoint(x: x, y: 200)) }
        if env["DEMO_MINIMIZE"] != nil { window.miniaturize(nil) }
        if env["DEMO_STATIC"] != nil { return }
        Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { _ in
            self.count += 1
            self.label.stringValue = "Count \(self.count)"
            NSAnimationContext.runAnimationGroup { ctx in
                ctx.duration = 0.4
                self.bar.animator().frame.origin.x = CGFloat((self.count % 5) * 100)
            }
        }
    }
}

let app = NSApplication.shared
let delegate = Delegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
