/// Moving between a page's rows by their place in the tree, given only each row's depth in page
/// order: VoiceOver's 「同级」 (Siblings) and 「父级」 (Parent) rotors, and how many children a row
/// shows (wiki: iOS-Cross-Platform §4.4).
public enum RowNavigation {
    /// The next (or previous) row at the same depth under the same parent; nil at the end of
    /// the parent's children.
    public static func sibling(of index: Int, in depths: [Int], forward: Bool) -> Int? {
        guard depths.indices.contains(index) else { return nil }
        let depth = depths[index]
        var i = index + (forward ? 1 : -1)
        while depths.indices.contains(i) {
            if depths[i] == depth { return i }
            if depths[i] < depth { return nil }
            i += forward ? 1 : -1
        }
        return nil
    }

    /// The row this one sits under; nil for a row at the top of the page.
    public static func parent(of index: Int, in depths: [Int]) -> Int? {
        guard depths.indices.contains(index) else { return nil }
        let depth = depths[index]
        var i = index - 1
        while i >= 0 {
            if depths[i] < depth { return i }
            i -= 1
        }
        return nil
    }

    /// How many rows sit directly under this one on the page (none for a collapsed row).
    public static func childCount(of index: Int, in depths: [Int]) -> Int {
        guard depths.indices.contains(index) else { return 0 }
        let depth = depths[index]
        var count = 0
        var i = index + 1
        while i < depths.count, depths[i] > depth {
            if depths[i] == depth + 1 { count += 1 }
            i += 1
        }
        return count
    }
}
