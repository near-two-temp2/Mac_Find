import XCTest
@testable import SearchEngine

final class SearchEngineTests: XCTestCase {

    /// Build a small in-memory index from a fixed path list, then round-trip it
    /// through mmap and exercise the full two-phase search.
    private func makeIndex(_ paths: [(String, Bool)]) throws -> (URL, SearchEngine) {
        let tmp = FileManager.default.temporaryDirectory
            .appendingPathComponent("mhf-b-\(UUID().uuidString).idx")
        let builder = IndexBuilder()
        try builder.build(paths: paths.map { (path: $0.0, isDir: $0.1) }, outputURL: tmp)
        let engine = try SearchEngine(indexURL: tmp)
        return (tmp, engine)
    }

    func testBitmaskEncoding() {
        var m: UInt64 = 0
        Bitmask.addByte(0x61, to: &m) // 'a'
        XCTAssertEqual(m & 1, 1)
        let qm = Bitmask.compute(Array("abc".utf8))
        let em = Bitmask.compute(Array("xabcy".utf8))
        XCTAssertTrue(Bitmask.contains(entry: em, query: qm))
        let miss = Bitmask.compute(Array("xyz".utf8))
        XCTAssertFalse(Bitmask.contains(entry: miss, query: qm))
    }

    func testRoundTripAndBitmaskPrefilter() throws {
        let (url, engine) = try makeIndex([
            ("/Users/x/Documents/report.pdf", false),
            ("/Users/x/Downloads/photo.jpg", false),
            ("/Users/x/Projects/machaifind/main.swift", false),
            ("/Users/x/Projects", true),
        ])
        defer { try? FileManager.default.removeItem(at: url) }

        XCTAssertEqual(engine.index.count, 4)

        let hits = engine.search("report")
        XCTAssertFalse(hits.isEmpty)
        XCTAssertTrue(hits.first!.path.hasSuffix("report.pdf"))
    }

    func testFuzzyMatchAndRanking() throws {
        let (url, engine) = try makeIndex([
            ("/a/machaifind_main.swift", false),
            ("/a/random_notes.txt", false),
            ("/a/msf.txt", false),          // contains m,s,f subsequence
            ("/a/main.swift", false),
        ])
        defer { try? FileManager.default.removeItem(at: url) }

        // "main.swift" should rank the exact basename above fuzzy scatter.
        let hits = engine.search("mainswift")
        XCTAssertFalse(hits.isEmpty)
        XCTAssertTrue(hits.first!.path.hasSuffix("main.swift") ||
                      hits.first!.path.hasSuffix("machaifind_main.swift"))
    }

    func testTypeFilters() throws {
        let (url, engine) = try makeIndex([
            ("/a/folderthing", true),
            ("/a/filething.txt", false),
        ])
        defer { try? FileManager.default.removeItem(at: url) }

        let dirs = engine.search("thing", options: SearchOptions(dirsOnly: true))
        XCTAssertTrue(dirs.allSatisfy { $0.isDir })
        let files = engine.search("thing", options: SearchOptions(filesOnly: true))
        XCTAssertTrue(files.allSatisfy { !$0.isDir })
    }

    func testExtensionFilter() throws {
        let (url, engine) = try makeIndex([
            ("/a/alpha.swift", false),
            ("/a/alpha.txt", false),
        ])
        defer { try? FileManager.default.removeItem(at: url) }

        let hits = engine.search("alpha", options: SearchOptions(ext: "swift"))
        XCTAssertEqual(hits.count, 1)
        XCTAssertTrue(hits.first!.path.hasSuffix(".swift"))
    }

    func testEmptyQueryBrowses() throws {
        let (url, engine) = try makeIndex([
            ("/a/one", false), ("/a/two", false), ("/a/three", false),
        ])
        defer { try? FileManager.default.removeItem(at: url) }
        let hits = engine.search("", options: SearchOptions(maxResults: 2))
        XCTAssertEqual(hits.count, 2)
    }

    func testSimdFindByte() {
        let text = Array("xxxxxxxxxxxxxxxxxABCxxx".utf8) // 'A' past a 16-byte chunk
        text.withUnsafeBufferPointer { buf in
            let idx = FuzzyScorer.simdFindByte(buf.baseAddress!, count: buf.count, needle: 0x41, from: 0)
            XCTAssertEqual(idx, 17)
            let none = FuzzyScorer.simdFindByte(buf.baseAddress!, count: buf.count, needle: 0x5A, from: 0)
            XCTAssertEqual(none, -1)
        }
    }
}
