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

    func testBoundaryBonusRanksAppManagerFirst() throws {
        let s = try buildSearcher()
        let hits = s.search("am")
        let app = hits.firstIndex { $0.path.hasSuffix("AppManager.swift") }
        let team = hits.firstIndex { $0.path.hasSuffix("teamwork.txt") }
        if let a = app, let t = team { XCTAssertLessThan(a, t) }
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
