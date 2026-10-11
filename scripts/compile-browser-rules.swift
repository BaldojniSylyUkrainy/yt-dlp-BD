// No webview, page or UI is created. Compile the exact production JSON using
// the OS WebKit compiler, which accepts a smaller regex language than Rust/JS.
import Foundation
import WebKit

guard CommandLine.arguments.count == 2 else { exit(2) }
WKContentRuleListStore.default().compileContentRuleList(
    forIdentifier: "baldoj-local-network-regression",
    encodedContentRuleList: CommandLine.arguments[1]
) { list, error in
    guard list != nil, error == nil else {
        fputs("Native content rules rejected: \(String(describing: error))\n", stderr)
        exit(1)
    }
    print("Native content rules compiled")
    exit(0)
}
DispatchQueue.main.asyncAfter(deadline: .now() + 15) { exit(3) }
RunLoop.main.run()
