import SwiftUI

// Workaround: SwiftUI's .toolbar { } is ambiguous between ToolbarContent
// and CustomizableToolbarContent overloads (Xcode 16+). This wrapper
// constrains to ToolbarContent only, resolving the ambiguity.
extension View {
    func appToolbar<C: ToolbarContent>(@ToolbarContentBuilder _ content: () -> C) -> some View {
        toolbar(content: content)
    }
}
