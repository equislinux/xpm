//! Reverse dependency ordering for removals.

use super::*;

pub(super) fn sort_removal(
    installed: &[InstalledPkg],
    names: &[&str],
    ignore_versions: bool,
) -> Vec<String> {
    if names.is_empty() {
        return Vec::new();
    }

    let by_name: HashMap<&str, &InstalledPkg> = installed
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect();

    let edges: Vec<Vec<usize>> = names
        .iter()
        .map(|name| {
            let package = by_name.get(name).copied();
            names
                .iter()
                .enumerate()
                .filter_map(|(index, candidate)| {
                    let candidate = by_name.get(candidate).copied()?;
                    let package = package?;
                    package
                        .depends
                        .iter()
                        .any(|dependency| satisfies(candidate, dependency, ignore_versions))
                        .then_some(index)
                })
                .collect()
        })
        .collect();

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Unvisited,
        Processing,
        Done,
    }

    let mut state = vec![State::Unvisited; names.len()];
    let mut post_order = Vec::new();

    for root in 0..names.len() {
        if state[root] != State::Unvisited {
            continue;
        }

        let mut stack = vec![(root, 0usize)];
        state[root] = State::Processing;

        while let Some(&(node, next)) = stack.last() {
            if next < edges[node].len() {
                let child = edges[node][next];
                stack.last_mut().expect("stack is not empty").1 += 1;

                if state[child] == State::Unvisited {
                    state[child] = State::Processing;
                    stack.push((child, 0));
                }
            } else {
                state[node] = State::Done;
                post_order.push(names[node].to_string());
                stack.pop();
            }
        }
    }

    post_order.reverse();
    post_order
}
