use std::{process::Command, collections::{HashMap, HashSet}};


#[derive(Debug)]
pub struct Pairing {
    pub filename1: String,
    pub filename2: String,
    pub score: f64,
}

#[derive(Debug)]
pub struct CompareTask {
    pub filename1: String,
    pub filename2: String,
}

pub fn run_executable(pairings: &Vec<Pairing>, executable: &Option<(&str, Vec<&str>)>) {
    let groups = make_groups(pairings);
    for group in groups {
        let line = group.join(" ");
        println!("{}", line);
        if let Some((program, args)) = &executable {
            Command::new(program)
                .args(args)
                .args(group)
                .output()
                .expect("Unable to run program provided");
        }
    }
}

pub fn make_groups<'a>(pairs: &Vec<Pairing>) -> Vec<Vec<String>> {
    let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();

    // Build the graph
    for pair in pairs.iter() {
        // let (node1, node2) = *pair;
        let node1 = &pair.filename1;
        let node2 = &pair.filename2;
        graph.entry(node1).or_insert(vec![]).push(node2);
        graph.entry(node2).or_insert(vec![]).push(node1);
    }

    let mut visited: HashSet<&str> = HashSet::new();
    let mut groups: Vec<Vec<String>> = Vec::new();

    // Perform DFS to find connected components
    fn dfs<'a>(
        node: &'a str,
        graph: &'a HashMap<&str, Vec<&str>>,
        visited: &mut HashSet<&'a str>,
        mut group: Vec<String>,
    ) -> Vec<String> {
        visited.insert(node);
        group.push(node.to_string());
        if let Some(neighbors) = graph.get(node) {
            for &neighbor in neighbors.iter() {
                if !visited.contains(neighbor) {
                    group = dfs(neighbor, graph, visited, group);
                }
            }
        }
        group
    }

    for node in graph.keys() {
        if !visited.contains(node) {
            let mut group: Vec<String> = Vec::new();
            group = dfs(node, &graph, &mut visited, group);
            groups.push(group);
        }
    }

    groups
}