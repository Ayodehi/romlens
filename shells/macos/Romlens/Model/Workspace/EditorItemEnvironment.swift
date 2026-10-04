import SwiftUI

extension EnvironmentValues {
    /// The tab a view is shown in (docs/29): which scroll requests it acts
    /// on, and whose C and graph it shows. Nil outside a tab.
    @Entry var editorItem: UUID? = nil
}
