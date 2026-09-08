import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    RegisterGeneratedPlugins(registry: flutterViewController)
    self.title = "BDJ Studio Search Pro"

    super.awakeFromNib()

    let args = CommandLine.arguments
    if args.contains("--startup") || args.contains("--tray") || args.contains("--minimized") {
      self.setIsVisible(false)
      self.orderOut(nil)
    }
  }
}
