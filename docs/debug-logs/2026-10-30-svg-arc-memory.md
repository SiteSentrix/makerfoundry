# 2026-10-30: an SVG with a huge arc made the reader allocate gigabytes

**Symptom.** `cargo xtask ci` failed once with only "test failed, to rerun pass `-p makerfoundry
--test untrusted_input`": no panic, no failing case saved, and the next runs passed.

**Reproduce.** The kernel log (`dmesg`) showed the container's memory limit killing
`untrusted_input` at 13.8 GB. Each property test was then run on its own, with 20,000 cases and
`ulimit -v 3000000` so an allocation fails in the test rather than the kernel killing it: only
`svg_generated` died, with "memory allocation of 3758096384 bytes failed".

**Where.** The backtrace: `laser::read_svg` → `usvg::Tree::from_str` → usvg's path conversion →
`kurbo::Arc::to_cubic_beziers`. The generated SVGs use numbers like `1e30`; an arc that size is
cut into an enormous number of curves inside usvg, before MakerFoundry's own limits
(`MAX_POINTS`, `MAX_SIZE_MM`) are checked on what usvg returns. A single such arc stays under a
few GB; several in one path inside groups ran away. Any uploaded SVG could do the same on the
server.

**Fix.** `read_svg` now parses the XML once itself (usvg's own `roxmltree`, with the same
options `usvg::Tree::from_str` uses), refuses the file if any number in a drawing attribute
(`d`, `points`, coordinates, radii, sizes, `viewBox`, transforms, `style`) or a `<style>` sheet
is beyond ±1e7 (`MAX_SVG_NUMBER`: far beyond any laser bed in any unit), then gives the same
document to `usvg::Tree::from_xmltree`. Ids, links and editors' namespaced attributes aren't
drawing and aren't checked, so a file named `logo_20231231.svg` still reads.

**Verified.** With the fix, `svg_generated` at 20,000 cases under the 3 GB cap passes in 37 s;
without it, the same run doesn't finish in 5 minutes. The whole property-test file went from
6 to 9 s to under 1 s. Unit tests: `laser::tests::numbers_in_text_are_found_and_measured`
and `an_svg_with_huge_numbers_is_refused_before_usvg_reads_it`.

**How it got in.** The readers' limits were all checked on usvg's output; nothing bounded the
work usvg does to produce it. The safeguard is the check before usvg, and the property tests,
which found it. A crash with no saved case is what an out-of-memory kill looks like: check
`dmesg` first.
