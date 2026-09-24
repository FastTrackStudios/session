import SwiftUI

extension Color {
    /// 0RGB → Color.
    init(rgb: UInt32) {
        self.init(
            red: Double((rgb >> 16) & 0xFF) / 255,
            green: Double((rgb >> 8) & 0xFF) / 255,
            blue: Double(rgb & 0xFF) / 255)
    }
}

extension WatchSection {
    var tint: Color { color == 0 ? Color(rgb: 0x9CA3AF) : Color(rgb: color) }
}
