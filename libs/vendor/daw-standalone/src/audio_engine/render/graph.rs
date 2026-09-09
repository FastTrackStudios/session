//! Routing-graph analysis run once per snapshot (not per block):
//! topological processing order + solo routing mask.

use std::collections::VecDeque;

use super::snapshot::TrackSnapshot;

/// Topological processing order over the ROUTING GRAPH: a track must
/// be fully processed before anything it feeds — its folder parent
/// (children sum upward) and every send destination (busses receive
/// post-fader signal). Depth ordering alone breaks bus mixes: a send
/// landing on an already-processed bus would never be forwarded
/// (the session's whole drum mix flows Drums → DRUM BUS → MIX BUS).
/// Kahn's algorithm; any cycle remainder (feedback loops) appends in
/// index order.
pub(crate) fn topo_order(tracks: &[TrackSnapshot]) -> Vec<usize> {
    let n = tracks.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    let mut edge = |a: usize, b: usize| {
        if a != b {
            adj[a].push(b);
            indeg[b] += 1;
        }
    };
    for (i, t) in tracks.iter().enumerate() {
        if let Some(pi) = t.parent_idx {
            edge(i, pi);
        }
        for snd in &t.sends {
            if let Some(di) = snd.dest_idx {
                edge(i, di);
            }
        }
    }
    let mut order: Vec<usize> = Vec::with_capacity(n);
    let mut queue: VecDeque<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    while let Some(i) = queue.pop_front() {
        order.push(i);
        for &j in &adj[i] {
            indeg[j] -= 1;
            if indeg[j] == 0 {
                queue.push_back(j);
            }
        }
    }
    if order.len() < n {
        for i in 0..n {
            if indeg[i] > 0 {
                order.push(i);
            }
        }
    }
    order
}

/// Solo routing: with folder summing, a soloed track's signal flows
/// THROUGH its ancestors — they (and its descendants) must keep
/// passing audio even though they aren't soloed themselves. Returns
/// `None` when nothing is soloed (everything passes).
pub(crate) fn solo_pass(tracks: &[TrackSnapshot]) -> Option<Vec<bool>> {
    if !tracks.iter().any(|t| t.soloed) {
        return None;
    }
    let mut pass = vec![false; tracks.len()];
    for (i, t) in tracks.iter().enumerate() {
        if !t.soloed {
            continue;
        }
        // The soloed track + ancestors.
        pass[i] = true;
        let mut cur = i;
        while let Some(pi) = tracks[cur].parent_idx {
            if pass[pi] {
                break;
            }
            pass[pi] = true;
            cur = pi;
        }
        // Descendants (soloing a folder solos its children).
        for j in 0..tracks.len() {
            let mut cur = j;
            let mut hops = 0;
            while let Some(pi) = tracks[cur].parent_idx {
                if hops >= 64 {
                    break;
                }
                if pi == i {
                    pass[j] = true;
                    break;
                }
                hops += 1;
                cur = pi;
            }
        }
    }
    Some(pass)
}
