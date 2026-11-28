// Writes the iOS app's string catalog the way Xcode does, so an Xcode build leaves it unchanged:
// Foundation's pretty-printed JSON with sorted keys, and every string marked
// `"extractionState" : "manual"` (strings are added by hand; Xcode marks strings it can't find in
// the app's own sources, such as FlowieKit's, "stale" otherwise). Run it after editing the catalog.
// usage: swift scripts/format-xcstrings.swift [--check] [catalog.xcstrings]
//        (default: apple/FlowieApp/Resources/Localizable.xcstrings; --check only reports, exit 1
//        if the file would change)
import Foundation

var args = Array(CommandLine.arguments.dropFirst())
let check = args.first == "--check"
if check { args.removeFirst() }
let path = args.first ?? "apple/FlowieApp/Resources/Localizable.xcstrings"
let url = URL(fileURLWithPath: path)

let original = try Data(contentsOf: url)
guard var catalog = try JSONSerialization.jsonObject(with: original) as? [String: Any],
      var strings = catalog["strings"] as? [String: Any]
else {
    FileHandle.standardError.write(Data("\(path): not a string catalog\n".utf8))
    exit(1)
}
for (key, value) in strings {
    var entry = value as? [String: Any] ?? [:]
    entry["extractionState"] = "manual"
    strings[key] = entry
}
catalog["strings"] = strings
let formatted = try JSONSerialization.data(
    withJSONObject: catalog, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])

if formatted == original { exit(0) }
if check {
    FileHandle.standardError.write(Data("\(path) isn't in Xcode's format; run swift scripts/format-xcstrings.swift\n".utf8))
    exit(1)
}
try formatted.write(to: url)
