//! Shared thread-local tree-sitter parser for Elixir sources.
//!
//! Every previous call site built a fresh `tree_sitter::Parser` and set the
//! pinned grammar per parse. Reusing one parser per thread keeps the grammar
//! assignment and internal buffers across files handled by the same worker.

use std::cell::RefCell;

thread_local! {
    static PARSER: RefCell<Option<tree_sitter::Parser>> = const { RefCell::new(None) };
}

/// Parse `source` with the pinned grammar, reusing the calling thread's parser.
///
/// Returns `None` when the grammar is unavailable or parsing yields nothing.
/// The returned tree is fully owned and independent of the cached parser, so
/// the parser can be reused immediately for the next file.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
#[must_use]
pub(crate) fn parse(source: &str) -> Option<tree_sitter::Tree> {
    PARSER
        .try_with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                let mut fresh = tree_sitter::Parser::new();
                if fresh
                    .set_language(&tree_sitter_elixir::LANGUAGE.into())
                    .is_err()
                {
                    return None;
                }
                *slot = Some(fresh);
            }
            slot.as_mut()?.parse(source, None)
        })
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_parse(source: &str) -> Option<tree_sitter::Tree> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_elixir::LANGUAGE.into())
            .ok()?;
        parser.parse(source, None)
    }

    #[test]
    fn reused_parser_matches_fresh_parse() {
        for source in [
            "x = 1\n",
            "defmodule M do\n  def foo(x), do: x\nend\n",
            "def foo( do\n",
            "",
        ] {
            let shared = parse(source)
                .map(|tree| (tree.root_node().has_error(), tree.root_node().to_sexp()));
            let fresh = fresh_parse(source)
                .map(|tree| (tree.root_node().has_error(), tree.root_node().to_sexp()));
            assert_eq!(shared, fresh, "{source:?}");
        }
    }

    #[test]
    fn error_parse_does_not_poison_next_parse() {
        let bad = parse("def foo( do\n").expect("error tree still parses");
        assert!(bad.root_node().has_error());
        let good = parse("x = 1\n").expect("next parse works");
        assert!(!good.root_node().has_error());
        assert_eq!(
            good.root_node().to_sexp(),
            fresh_parse("x = 1\n")
                .expect("fresh parses")
                .root_node()
                .to_sexp()
        );
    }

    #[test]
    fn concurrent_threads_parse_independently() {
        let sources = [
            "defmodule A do\n  def f, do: 1\nend\n",
            "x = [1, 2, 3]\n",
            "def broken( do\n",
        ];
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for _ in 0..8 {
                handles.push(scope.spawn(|| {
                    for source in sources {
                        let shared = parse(source)
                            .map(|tree| (tree.root_node().has_error(), tree.root_node().to_sexp()));
                        let fresh = fresh_parse(source)
                            .map(|tree| (tree.root_node().has_error(), tree.root_node().to_sexp()));
                        assert_eq!(shared, fresh, "{source:?}");
                    }
                }));
            }
            for handle in handles {
                handle.join().expect("worker panicked");
            }
        });
    }
}
