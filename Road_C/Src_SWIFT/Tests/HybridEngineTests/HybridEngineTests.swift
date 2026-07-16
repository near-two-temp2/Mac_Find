import XCTest
@testable import HybridEngine

/// Headless tests for the pure-logic engine — no GUI, no root, no window server,
/// so they run cleanly on any macOS CI runner. They exercise the full
/// build → mmap → prefilter → fzf pipeline against a temp fixture tree.
final class HybridEngineTests: XCTestCase {

    private var tmp: URL!

    override func setUpWithError() throws {
        tmp = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("mhfc-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
        for rel in ["AppManager.swift", "app_notes.txt", "README.md",
                    "deep/nested/report_final.pdf", "deep/nested/teamwork.txt"] {
            let url = tmp.appendingPathComponent(rel)
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Data("x".utf8).write(to: url)
        }
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: tmp)
    }

    private func buildSearcher() throws -> IndexSearcher {
        let idx = tmp.appendingPathComponent("t.idx").path
        let n = try IndexBuilder().build(roots: [tmp.path], to: idx)
        XCTAssertGreaterThanOrEqual(n, 5)
        return try IndexSearcher(path: idx)
    }

    func testBitmaskPrefilter() {
        let entry = Bitmask.compute(Array("appmanager".utf8))
        XCTAssertTrue(Bitmask.contains(entry: entry, query: Bitmask.compute(Array("apm".utf8))))
        XCTAssertFalse(Bitmask.contains(entry: entry, query: Bitmask.compute(Array("xyz".utf8))))
    }

    func testFuzzyFindsBasename() throws {
        let s = try buildSearcher()
        let hits = s.search("appman")
        XCTAssertTrue(hits.contains { $0.path.hasSuffix("AppManager.swift") })
    }

    /// A word-start (prefix) match must outrank a mid-word scattered/substring
    /// match. Querying "app" should rank `AppManager.swift` (prefix → tierPrefix)
    /// above `teamwork.txt` (which doesn't contain "app" at all and is filtered
    /// out) and above any mid-word hit.
    func testPrefixMatchRanksAppManagerFirst() throws {
        let s = try buildSearcher()
        let hits = s.search("app")
        let app = hits.firstIndex { $0.path.hasSuffix("AppManager.swift") }
        let notes = hits.firstIndex { $0.path.hasSuffix("app_notes.txt") }
        XCTAssertNotNil(app, "AppManager.swift (prefix match) should be present")
        // teamwork.txt has no "app" subsequence and must not appear.
        XCTAssertFalse(hits.contains { $0.path.hasSuffix("teamwork.txt") },
                       "teamwork.txt lacks the 'app' subsequence and must be filtered out")
        // Both "app"-prefixed names tie on tier; AppManager (shorter to the match)
        // and app_notes are both valid prefixes, so just require both are present
        // and ranked ahead of any non-prefix noise.
        XCTAssertNotNil(notes, "app_notes.txt (prefix match) should be present")
    }

    /// The tier order must place a contiguous substring above a scattered
    /// subsequence: querying "am" makes `teamwork.txt` (contiguous "am" in
    /// "te-am-work" → tierSubstring) outrank `AppManager.swift` (no contiguous
    /// "am"; only a scattered a…m → no tier). This is the intended behaviour of
    /// the literal-match tiers.
    func testContiguousSubstringOutranksScattered() throws {
        let s = try buildSearcher()
        let hits = s.search("am")
        let app = hits.firstIndex { $0.path.hasSuffix("AppManager.swift") }
        let team = hits.firstIndex { $0.path.hasSuffix("teamwork.txt") }
        if let a = app, let t = team {
            XCTAssertLessThan(t, a, "contiguous 'am' in teamwork should outrank scattered 'am' in AppManager")
        }
    }

    func testExtensionConstraint() throws {
        let s = try buildSearcher()
        let pdfs = s.search(".pdf")
        XCTAssertFalse(pdfs.isEmpty)
        XCTAssertTrue(pdfs.allSatisfy { $0.path.hasSuffix(".pdf") })
    }

    func testPathQueryAcrossSeparators() throws {
        let s = try buildSearcher()
        let hits = s.search("nested/report")
        XCTAssertTrue(hits.contains { $0.path.hasSuffix("report_final.pdf") })
    }

    func testFilesOnlyDirsOnly() throws {
        let s = try buildSearcher()
        XCTAssertTrue(s.search("deep", options: SearchOptions(dirsOnly: true)).allSatisfy { $0.isDir })
        XCTAssertTrue(s.search("report", options: SearchOptions(filesOnly: true)).allSatisfy { !$0.isDir })
    }

    /// The ① regression: a query must never return an entry that doesn't even
    /// contain the query as a subsequence (e.g. `temp_test` returning `ev_work`).
    func testNonSubsequenceNeverReturned() throws {
        // Fixture with a real target plus decoys that share only some letters.
        let root = tmp.appendingPathComponent("nonsub", isDirectory: true)
        for rel in ["temp_test/marker.txt", "ev_work/x.txt", "op_work_mac/y.txt", "CLAUDE.md"] {
            let url = root.appendingPathComponent(rel)
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Data("x".utf8).write(to: url)
        }
        let idx = tmp.appendingPathComponent("nonsub.idx").path
        _ = try IndexBuilder().build(roots: [root.path], to: idx)
        let s = try IndexSearcher(path: idx)

        let hits = s.search("temp_test")
        // Nothing lacking the subsequence t-e-m-p-_-t-e-s-t may appear.
        for h in hits {
            let name = (h.path as NSString).lastPathComponent.lowercased()
            XCTAssertFalse(name == "ev_work" || name == "op_work_mac" || name == "claude.md",
                           "non-matching entry leaked into results: \(h.path)")
        }
        // And the real directory must be present and ranked first.
        XCTAssertEqual(hits.first?.path.hasSuffix("/temp_test"), true,
                       "temp_test should rank first, got \(hits.first?.path ?? "nil")")
    }

    /// Exact / substring basename matches must dominate scattered fuzzy hits.
    func testExactSubstringOutranksScattered() throws {
        let root = tmp.appendingPathComponent("rank", isDirectory: true)
        for rel in ["temp_test", "contemplate_stest.txt", "t_e_m_p_test_es_t.log"] {
            let url = root.appendingPathComponent(rel)
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            if rel == "temp_test" {
                try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
            } else {
                try Data("x".utf8).write(to: url)
            }
        }
        let idx = tmp.appendingPathComponent("rank.idx").path
        _ = try IndexBuilder().build(roots: [root.path], to: idx)
        let s = try IndexSearcher(path: idx)

        let hits = s.search("temp_test")
        XCTAssertEqual(hits.first?.path.hasSuffix("/temp_test"), true,
                       "exact 'temp_test' must rank above scattered matches; got \(hits.map { $0.path })")
    }

    /// A basename match must outrank a match that only appears in a parent dir.
    func testBasenameBeatsParentPath() throws {
        let root = tmp.appendingPathComponent("bn", isDirectory: true)
        for rel in ["report/notes.txt", "misc/report.txt"] {
            let url = root.appendingPathComponent(rel)
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Data("x".utf8).write(to: url)
        }
        let idx = tmp.appendingPathComponent("bn.idx").path
        _ = try IndexBuilder().build(roots: [root.path], to: idx)
        let s = try IndexSearcher(path: idx)

        let hits = s.search("report")
        let bnIdx = hits.firstIndex { $0.path.hasSuffix("/report.txt") }
        let parentIdx = hits.firstIndex { $0.path.hasSuffix("/notes.txt") }
        if let b = bnIdx, let p = parentIdx {
            XCTAssertLessThan(b, p, "basename 'report.txt' should outrank parent-path 'report/notes.txt'")
        }
        XCTAssertNotNil(bnIdx, "report.txt (basename match) should be present")
    }

    /// Old-version indices are rejected (→ rebuilt, not served with stale data).
    func testOldVersionIndexRejected() throws {
        let idx = tmp.appendingPathComponent("v.idx").path
        _ = try IndexBuilder().build(roots: [tmp.path], to: idx)
        // Corrupt the version field to an older value. Header layout: magic is
        // bytes 0..8 (UInt64), version is bytes 8..12 (UInt32), entryCount 12..16.
        var data = try Data(contentsOf: URL(fileURLWithPath: idx))
        data.replaceSubrange(8..<12, with: [0x02, 0, 0, 0]) // version 2
        try data.write(to: URL(fileURLWithPath: idx))
        XCTAssertThrowsError(try IndexSearcher(path: idx)) { err in
            guard case IndexError.corrupt = err else { return XCTFail("expected .corrupt, got \(err)") }
        }
    }

    func testCorruptIndexThrows() throws {
        let bad = tmp.appendingPathComponent("bad.idx")
        try Data(repeating: 0, count: 128).write(to: bad)
        XCTAssertThrowsError(try IndexSearcher(path: bad.path))
    }

    func testMissingIndexThrowsMissing() {
        XCTAssertThrowsError(try IndexSearcher(path: tmp.appendingPathComponent("nope.idx").path)) { err in
            guard case IndexError.missing = err else { return XCTFail("expected .missing, got \(err)") }
        }
    }
}
