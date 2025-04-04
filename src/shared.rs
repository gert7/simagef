use std::{
    collections::{HashMap, HashSet},
    process::Command,
};

#[derive(Debug)]
pub struct CompareTask {
    pub index1: usize,
    pub index2: usize,
}

#[derive(Debug)]
pub struct Pairing {
    pub index1: usize,
    pub index2: usize,
    pub score: f64,
}

pub fn make_groups_and_exec(
    name_map: &[String],
    pairings: Vec<Pairing>,
    executable: &Option<(&str, Vec<&str>)>,
) {
    let groups = make_groups(pairings);
    for group in groups {
        let name_group: Vec<String> = group.iter().map(|index| {
            name_map[*index].clone()
        }).collect();
        let line = name_group.join(" ");
        println!("{}", line);
        if let Some((program, args)) = &executable {
            Command::new(program)
                .args(args)
                .args(name_group)
                .output()
                .expect("Unable to run program provided");
        }
    }
}

pub fn make_groups(pairs: Vec<Pairing>) -> Vec<Vec<usize>> {
    let mut graph: HashMap<usize, Vec<usize>> = HashMap::new();

    // Build the graph
    for pair in pairs.iter() {
        // let (node1, node2) = *pair;
        let node1 = pair.index1;
        let node2 = pair.index2;
        graph.entry(node1).or_default().push(node2);
        graph.entry(node2).or_default().push(node1);
    }

    let mut visited: HashSet<usize> = HashSet::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();

    // Perform DFS to find connected components
    fn dfs(
        node: usize,
        graph: &HashMap<usize, Vec<usize>>,
        visited: &mut HashSet<usize>,
        mut group: Vec<usize>,
    ) -> Vec<usize> {
        visited.insert(node);
        group.push(node);
        if let Some(neighbors) = graph.get(&node) {
            for neighbor in neighbors.iter() {
                if !visited.contains(neighbor) {
                    group = dfs(*neighbor, graph, visited, group);
                }
            }
        }
        group
    }

    for node in graph.keys() {
        if !visited.contains(node) {
            let mut group: Vec<usize> = Vec::new();
            group = dfs(*node, &graph, &mut visited, group);
            groups.push(group);
        }
    }

    groups
}
