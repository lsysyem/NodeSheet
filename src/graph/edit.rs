//! 节点图上的编辑操作，照 Houdini 的网络编辑器：多选（Shift / Ctrl 点、框选）、删节点时把下游接回上游、
//! 复制粘贴（Alt 拖复制）、剪断连线（Y）、把节点拖到线上插进去、沿着链选上下游（PgUp / PgDn）、自动排列（L）。

use super::*;

impl Graph {
    /// 选中的全部节点：当前节点（参数面板编辑的那个）在最前，然后是另外选上的。
    pub fn selection(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.selected.filter(|&id| self.node(id).is_some()).into_iter().collect();
        for &id in &self.picked {
            if !ids.contains(&id) && self.node(id).is_some() {
                ids.push(id);
            }
        }
        ids
    }

    pub fn is_selected(&self, id: u32) -> bool {
        self.selected == Some(id) || self.picked.contains(&id)
    }

    /// 只选中这一个（None：一个也不选）。
    pub fn select_only(&mut self, id: Option<u32>) {
        self.selected = id;
        self.picked.clear();
        self.selected_wire = None;
    }

    /// 加选：它成为当前节点，原来的当前节点还选着。
    pub fn select_add(&mut self, id: u32) {
        if let Some(current) = self.selected.filter(|&c| c != id && !self.picked.contains(&c)) {
            self.picked.push(current);
        }
        self.picked.retain(|&p| p != id);
        self.selected = Some(id);
        self.selected_wire = None;
    }

    /// 减选。去掉的是当前节点时，当前节点换成还选着的第一个；表格级一个都不剩时留着它（参数面板总有个节点可看）。
    pub fn select_remove(&mut self, id: u32) {
        self.picked.retain(|&p| p != id);
        if self.selected == Some(id) {
            if !self.picked.is_empty() {
                self.selected = Some(self.picked.remove(0));
            } else if self.vop {
                self.selected = None;
            }
        }
    }

    pub fn select_all(&mut self) {
        let ids: Vec<u32> = self.nodes.iter().map(|n| n.id).collect();
        let current = self.selected.or(ids.first().copied());
        self.picked = ids.into_iter().filter(|&id| Some(id) != current).collect();
        self.selected = current;
        self.selected_wire = None;
    }

    /// 框选（画布坐标的矩形，节点本身碰到框就算）：mode 0 换成框里的 / 1 加上 / 2 去掉。
    /// 当前节点还在选中里就不换，否则换成框里最上面（表达式网络里最左边）的那个。
    pub fn select_box(&mut self, rect: (f32, f32, f32, f32), mode: i32) {
        let (x0, x1) = (rect.0.min(rect.2), rect.0.max(rect.2));
        let (y0, y1) = (rect.1.min(rect.3), rect.1.max(rect.3));
        let mut hits: Vec<&Node> = (self.nodes.iter())
            .filter(|n| {
                let (w, h) = self.size(n);
                n.x <= x1 && n.x + w >= x0 && n.y <= y1 && n.y + h >= y0
            })
            .collect();
        hits.sort_by(|a, b| {
            let key = |n: &Node| if self.vop { (n.x, n.y) } else { (n.y, n.x) };
            key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal)
        });
        let hits: Vec<u32> = hits.into_iter().map(|n| n.id).collect();
        self.selected_wire = None;
        match mode {
            1 => {
                for &id in &hits {
                    if !self.is_selected(id) {
                        self.picked.push(id);
                    }
                }
                if self.selected.is_none() && !hits.is_empty() {
                    self.select_add(hits[0]);
                }
            }
            2 => {
                for &id in &hits {
                    self.select_remove(id);
                }
            }
            _ => {
                let current = self.selected.filter(|c| hits.contains(c)).or(hits.first().copied());
                match current {
                    Some(c) => {
                        self.selected = Some(c);
                        self.picked = hits.into_iter().filter(|&id| id != c).collect();
                    }
                    // 框里什么都没有：表达式网络里全不选；表格级留着当前节点
                    None => self.select_only(if self.vop { None } else { self.selected }),
                }
            }
        }
    }

    /// 删掉节点（Houdini 的做法）：原来接它的下游改接它的第一个上游（连着线的第一个输入），流程不断开。
    pub fn delete(&mut self, id: u32) {
        let first = self.node(id).and_then(|n| n.inputs.iter().flatten().next().copied());
        for slot in self.nodes.iter_mut().filter(|n| n.id != id).flat_map(|n| &mut n.inputs) {
            if *slot == Some(id) {
                *slot = first;
            }
        }
        self.remove(id);
    }

    /// ids 连同它们全部的上游（up）或下游（!up）。
    pub fn with_relatives(&self, ids: &[u32], up: bool) -> Vec<u32> {
        let mut all = ids.to_vec();
        let mut i = 0;
        while i < all.len() {
            let id = all[i];
            let next: Vec<u32> = if up {
                self.node(id).map_or(Vec::new(), |n| n.inputs.iter().flatten().copied().collect())
            } else {
                self.nodes.iter().filter(|n| n.inputs.contains(&Some(id))).map(|n| n.id).collect()
            };
            for n in next {
                if !all.contains(&n) {
                    all.push(n);
                }
            }
            i += 1;
        }
        all
    }

    /// 沿着链走一步（PgUp / PgDn）：上游是当前节点第一个连着线的输入，下游是接它的节点里位置最靠前的。
    pub fn walk(&self, down: bool) -> Option<u32> {
        let current = self.node(self.selected?)?;
        if !down {
            return current.inputs.iter().flatten().next().copied();
        }
        let key = |n: &Node| if self.vop { (n.y, n.x) } else { (n.x, n.y) };
        (self.nodes.iter())
            .filter(|n| n.inputs.contains(&Some(current.id)))
            .min_by(|a, b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal))
            .map(|n| n.id)
    }

    /// 复制节点（复制粘贴用的 JSON）：节点连同它们用到的非内置类型的定义。vop 记着是哪一级的（不能贴到另一级）。
    pub fn copy_nodes(&self, ids: &[u32]) -> Option<Value> {
        let mut types = Map::new();
        let nodes: Vec<Value> =
            self.nodes.iter().filter(|n| ids.contains(&n.id)).map(|n| node_to_json(n, &mut types)).collect();
        (!nodes.is_empty()).then(|| json!({ "vop": self.vop, "types": types, "nodes": nodes }))
    }

    /// 粘贴 copy_nodes 复制的节点：新 id、名字不重复，它们之间的连线照旧；接着外面的输入，keep_outside 时
    /// （Alt 拖复制）还接着、否则断开。at：整体的左上角放在哪（None：原地）。贴好的成为选中的，返回它们的 id。
    pub fn paste(&mut self, v: &Value, at: Option<(f32, f32)>, keep_outside: bool) -> Vec<u32> {
        install_types(v);
        let mut pasted: Vec<Node> = Vec::new();
        let mut ids: Vec<(u32, u32)> = Vec::new(); // (原来的 id, 新 id)
        for item in v["nodes"].as_array().into_iter().flatten() {
            let Some(Ok(mut node)) = node_from_json(item, self.vop) else { continue };
            self.next_id += 1;
            ids.push((node.id, self.next_id));
            node.id = self.next_id;
            node.primary = false;
            // 名字：原来的去掉末尾的数字，接第一个没人用的序号
            let base = node.title.trim_end_matches(|c: char| c.is_ascii_digit()).to_string();
            let base = if base.is_empty() { node.kind.short_name().to_string() } else { base };
            let taken = |t: &str| self.nodes.iter().any(|n| n.title == t) || pasted.iter().any(|n| n.title == t);
            node.title = (1..).map(|i| format!("{base}{i}")).find(|t| !taken(t)).unwrap();
            pasted.push(node);
        }
        let (left, top) = pasted.iter().fold((f32::MAX, f32::MAX), |(l, t), n| (l.min(n.x), t.min(n.y)));
        let existing: Vec<u32> = self.nodes.iter().map(|n| n.id).collect();
        for node in &mut pasted {
            for slot in &mut node.inputs {
                *slot = match ids.iter().find(|(old, _)| Some(*old) == *slot) {
                    Some((_, new)) => Some(*new),
                    None if keep_outside => slot.filter(|id| existing.contains(id)),
                    None => None,
                };
            }
            if let Some((x, y)) = at {
                node.x += x - left;
                node.y += y - top;
            }
        }
        let new: Vec<u32> = pasted.iter().map(|n| n.id).collect();
        self.nodes.extend(pasted);
        self.selected = new.first().copied();
        self.picked = new.iter().skip(1).copied().collect();
        self.selected_wire = None;
        new
    }

    /// 复制一份（Alt 拖）：原地，接着外面的输入照旧接着。
    pub fn duplicate(&mut self, ids: &[u32]) -> Vec<u32> {
        match self.copy_nodes(ids) {
            Some(v) => self.paste(&v, None, true),
            None => Vec::new(),
        }
    }

    /// 剪断和这条折线（画布坐标，Y 拖出来的）相交的连线，返回剪了几条。
    pub fn cut_wires(&mut self, line: &[(f32, f32)]) -> usize {
        let mut hits = Vec::new();
        for node in &self.nodes {
            for slot in 0..node.inputs.len() {
                let Some(points) = self.wire_points(node, slot) else { continue };
                let crosses = points.windows(2).any(|w| line.windows(2).any(|c| segments_cross(w[0], w[1], c[0], c[1])));
                if crosses {
                    hits.push((node.id, slot));
                }
            }
        }
        for &(to, slot) in &hits {
            self.disconnect(to, slot);
        }
        hits.len()
    }

    /// 拖着的节点压在哪条线上（松手就插进去）：节点要有输入、而且还没连任何线。
    pub fn wire_under(&self, id: u32) -> Option<(u32, usize)> {
        let node = self.node(id).filter(|n| !n.inputs.is_empty() && n.inputs.iter().all(|i| i.is_none()))?;
        if self.nodes.iter().any(|n| n.inputs.contains(&Some(id))) {
            return None;
        }
        let (w, h) = self.size(node);
        let inside = |(x, y): (f32, f32)| x >= node.x && x <= node.x + w && y >= node.y && y <= node.y + h;
        for other in self.nodes.iter().filter(|n| n.id != id) {
            for slot in 0..other.inputs.len() {
                if self.wire_points(other, slot).is_some_and(|points| points.into_iter().any(inside)) {
                    return Some((other.id, slot));
                }
            }
        }
        None
    }

    /// 节点插进连线 (目标, 输入槽)：上游 → 节点的第一个输入，节点 → 原来的目标。
    pub fn insert_on_wire(&mut self, id: u32, wire: (u32, usize)) -> bool {
        let (to, slot) = wire;
        let Some(from) = self.node(to).and_then(|n| n.inputs.get(slot).copied().flatten()) else { return false };
        if from == id || to == id || !self.connect(from, id, 0) {
            return false;
        }
        self.connect(id, to, slot)
    }

    /// 自动排列（L）：ids 这些节点按上下游排成一层一层（表格级从上往下、表达式网络从左往右），
    /// 每层里按上游的位置排开、互不重叠；排好的整体左上角留在原来的地方。
    /// 表达式网络的输出节点放在最后一层，彼此的上下顺序不变（同一种输出按从上到下排先后，换了顺序结果就变了）。
    pub fn layout(&mut self, ids: &[u32]) {
        let set: Vec<u32> = self.nodes.iter().map(|n| n.id).filter(|id| ids.contains(id)).collect();
        if set.is_empty() {
            return;
        }
        // 层：在 set 里的上游最深的那层 + 1（图没有环，按 set 的顺序反复推到不变为止）
        let mut rank: Vec<usize> = vec![0; set.len()];
        loop {
            let mut changed = false;
            for (i, &id) in set.iter().enumerate() {
                let node = self.node(id).expect("在图里");
                let deepest = node.inputs.iter().flatten().filter_map(|up| set.iter().position(|s| s == up)).map(|j| rank[j] + 1).max();
                if let Some(r) = deepest.filter(|&r| r > rank[i]) {
                    rank[i] = r;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let is_output = |id: u32| self.vop && self.node(id).is_some_and(|n| !n.kind.role.is_empty());
        let last = rank.iter().copied().max().unwrap_or(0);
        let last = if set.iter().any(|&id| is_output(id)) {
            let inner = set.iter().zip(&rank).filter(|(id, _)| !is_output(**id)).map(|(_, r)| *r).max();
            inner.map_or(last, |r| r + 1)
        } else {
            last
        };
        for (i, &id) in set.iter().enumerate() {
            if is_output(id) {
                rank[i] = last;
            }
        }
        // 主轴（层）和交叉轴（层里排开的方向）：表格级是 y / x，表达式网络是 x / y
        let main_of = |n: &Node| if self.vop { n.x } else { n.y };
        let cross_of = |n: &Node| if self.vop { n.y } else { n.x };
        let origin_main = set.iter().filter_map(|&id| self.node(id)).map(main_of).fold(f32::MAX, f32::min);
        let origin_cross = set.iter().filter_map(|&id| self.node(id)).map(cross_of).fold(f32::MAX, f32::min);
        let mut placed: Vec<(u32, f32, f32)> = Vec::new(); // (id, 主轴, 交叉轴)
        for r in 0..=last {
            // 想放的位置：上游（已经排好的）的平均位置；没有上游的按原来的位置
            let mut members: Vec<(u32, f32, f32)> = (set.iter().zip(&rank))
                .filter(|(_, rr)| **rr == r)
                .map(|(&id, _)| {
                    let node = self.node(id).expect("在图里");
                    let ups: Vec<f32> = (node.inputs.iter().flatten())
                        .filter_map(|up| placed.iter().find(|(p, ..)| p == up).map(|(.., c)| *c))
                        .collect();
                    let want = if ups.is_empty() { cross_of(node) } else { ups.iter().sum::<f32>() / ups.len() as f32 };
                    (id, want, cross_of(node))
                })
                .collect();
            members.sort_by(|a, b| (a.1, a.2).partial_cmp(&(b.1, b.2)).unwrap_or(std::cmp::Ordering::Equal));
            // 输出节点之间保持原来的上下顺序：占着它们排到的那几个位置，按原来的位置重新分
            let slots: Vec<usize> = (0..members.len()).filter(|&k| is_output(members[k].0)).collect();
            let mut outputs: Vec<(u32, f32, f32)> = slots.iter().map(|&k| members[k]).collect();
            outputs.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
            for (k, out) in slots.into_iter().zip(outputs) {
                members[k] = out;
            }
            // 从前往后推开：表格级每个节点占 NODE_DX（连右边的名字），表达式网络按节点高度 + 30
            let mut end = f32::MIN;
            for (id, want, _) in members {
                let pos = want.max(end);
                let size = match self.node(id) {
                    Some(n) if self.vop => vop_h(n.inputs.len()) + 30.0 + VOP_LABEL_H,
                    _ => NODE_DX,
                };
                end = pos + size;
                let main = origin_main + r as f32 * if self.vop { VOP_DX } else { NODE_DY };
                placed.push((id, main, pos));
            }
        }
        // 整体挪回原来的左上角
        let shift = origin_cross - placed.iter().map(|p| p.2).fold(f32::MAX, f32::min);
        for (id, main, cross) in placed {
            let vop = self.vop;
            if let Some(node) = self.node_mut(id) {
                if vop {
                    (node.x, node.y) = (main, cross + shift);
                } else {
                    (node.x, node.y) = (cross + shift, main);
                }
            }
        }
    }
}

/// 线段 ab 和 cd 相交（含端点碰上）。
fn segments_cross(a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)) -> bool {
    let cross = |o: (f32, f32), p: (f32, f32), q: (f32, f32)| (p.0 - o.0) * (q.1 - o.1) - (p.1 - o.1) * (q.0 - o.0);
    let (d1, d2) = (cross(c, d, a), cross(c, d, b));
    let (d3, d4) = (cross(a, b, c), cross(a, b, d));
    d1 * d2 <= 0.0 && d3 * d4 <= 0.0 && !(d1 == 0.0 && d2 == 0.0)
}

#[cfg(test)]
mod tests {
    use super::super::tests::kind;
    use super::*;

    /// read → select → unique 一条链
    fn chain() -> (Graph, u32, u32, u32) {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let read = g.primary().unwrap().id;
        let select = g.add(&kind("select"));
        let unique = g.add(&kind("unique"));
        (g, read, select, unique)
    }

    #[test]
    fn selection_add_remove_and_box() {
        let (mut g, read, select, unique) = chain();
        g.select_only(Some(read));
        g.select_add(unique);
        assert_eq!(g.selection(), vec![unique, read]);
        // 去掉当前节点：换成还选着的
        g.select_remove(unique);
        assert_eq!(g.selection(), vec![read]);
        // 表格级去掉最后一个：参数面板还留着它
        g.select_remove(read);
        assert_eq!(g.selected, Some(read));
        // 框住上面两个（节点在 (24,16) 和 (24,86)，下一个在 156）
        g.select_box((0.0, 0.0, 200.0, 100.0), 0);
        assert_eq!(g.selection(), vec![read, select]);
        g.select_box((0.0, 150.0, 200.0, 200.0), 1);
        assert_eq!(g.selection(), vec![read, select, unique]);
        g.select_box((0.0, 0.0, 200.0, 30.0), 2);
        assert_eq!(g.selection(), vec![select, unique]);
        // 加新节点：只选它
        let new = g.add(&kind("select"));
        assert_eq!(g.selection(), vec![new]);
    }

    #[test]
    fn delete_reconnects_downstream() {
        let (mut g, read, select, unique) = chain();
        g.delete(select);
        assert_eq!(g.node(unique).unwrap().inputs, vec![Some(read)]);
        assert!(g.node(select).is_none());
    }

    #[test]
    fn copy_paste_keeps_inner_wires() {
        let (mut g, read, select, unique) = chain();
        g.node_mut(select).unwrap().set("columns", "\"a\"");
        let copied = g.copy_nodes(&[select, unique]).unwrap();
        let new = g.paste(&copied, Some((500.0, 0.0)), false);
        assert_eq!(new.len(), 2);
        let (s2, u2) = (g.node(new[0]).unwrap(), g.node(new[1]).unwrap());
        assert_eq!((s2.title.as_str(), u2.title.as_str()), ("select2", "unique2"));
        assert_eq!((s2.inputs[0], u2.inputs[0]), (None, Some(new[0])));
        assert_eq!(s2.value("columns"), "\"a\"");
        assert_eq!((s2.x, s2.y), (500.0, 0.0));
        assert_eq!(g.selection(), new);
        // Alt 拖复制：外面的输入还接着
        let dup = g.duplicate(&[select]);
        assert_eq!(g.node(dup[0]).unwrap().inputs, vec![Some(read)]);
        // 源节点复制出来不是源节点
        let r = g.duplicate(&[read]);
        assert!(!g.node(r[0]).unwrap().primary);
    }

    #[test]
    fn paste_copies_expression_networks() {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let expr = g.add(&kind("expr"));
        let inner = g.node(expr).unwrap().net.as_ref().unwrap().nodes.len();
        let dup = g.duplicate(&[expr]);
        assert_eq!(g.node(dup[0]).unwrap().net.as_ref().unwrap().nodes.len(), inner);
    }

    #[test]
    fn cut_and_insert_on_wires() {
        let (mut g, read, select, unique) = chain();
        // read (24,16) → select (24,86)：连线从 (80,54) 到 (80,86)，横着划一刀
        assert_eq!(g.cut_wires(&[(60.0, 70.0), (100.0, 70.0)]), 1);
        assert_eq!(g.node(select).unwrap().inputs, vec![None]);
        g.connect(read, select, 0);
        // 新节点拖到 select → unique 那条线上
        g.select_only(None);
        let fresh = g.add_at(&kind("unique"), Some((30.0, 110.0)));
        assert_eq!(g.wire_under(fresh), Some((unique, 0)));
        assert!(g.insert_on_wire(fresh, (unique, 0)));
        assert_eq!(g.node(fresh).unwrap().inputs, vec![Some(select)]);
        assert_eq!(g.node(unique).unwrap().inputs, vec![Some(fresh)]);
        // 连着线的节点不算
        assert_eq!(g.wire_under(select), None);
    }

    #[test]
    fn walk_and_relatives() {
        let (mut g, read, select, unique) = chain();
        g.select_only(Some(select));
        assert_eq!((g.walk(false), g.walk(true)), (Some(read), Some(unique)));
        assert_eq!(g.with_relatives(&[unique], true), vec![unique, select, read]);
        assert_eq!(g.with_relatives(&[read], false), vec![read, select, unique]);
    }

    #[test]
    fn layout_stacks_chains_and_branches() {
        let (mut g, read, select, unique) = chain();
        // 打乱位置，加一个分支
        g.select_only(Some(read));
        let side = g.add(&kind("select"));
        for (id, x, y) in [(read, 100.0, 50.0), (select, 900.0, 20.0), (unique, -40.0, 400.0), (side, 500.0, 500.0)] {
            let n = g.node_mut(id).unwrap();
            (n.x, n.y) = (x, y);
        }
        let all: Vec<u32> = g.nodes.iter().map(|n| n.id).collect();
        g.layout(&all);
        let pos = |id| (g.node(id).unwrap().x, g.node(id).unwrap().y);
        // 一层一层往下，同一条链对齐；两个分支在同一层、不重叠
        assert_eq!(pos(read).1 + NODE_DY, pos(select).1);
        assert_eq!(pos(select).1, pos(side).1);
        assert_eq!(pos(select).1 + NODE_DY, pos(unique).1);
        assert_eq!(pos(unique).0, pos(select).0);
        assert!((pos(select).0 - pos(side).0).abs() >= NODE_DX);
        // 左上角留在原来的地方
        assert_eq!(all.iter().map(|&id| pos(id).0).fold(f32::MAX, f32::min), -40.0);
        assert_eq!(pos(read).1, 20.0);
    }

    #[test]
    fn layout_keeps_output_order() {
        let mut g = Graph::for_file(Path::new("a.csv"), "");
        let expr = g.add(&kind("expr"));
        let net = g.node_mut(expr).unwrap().net.as_mut().unwrap();
        let col = net.nodes[0].id;
        let out1 = net.nodes[1].id;
        net.select_only(None);
        let out2 = net.add_at(&nodes::find_expr("output").unwrap(), Some((40.0, 300.0)));
        net.connect(col, out2, 0);
        // out1 在上、out2 在下，但 out2 的位置更靠左
        net.node_mut(out2).unwrap().x = -500.0;
        let all: Vec<u32> = net.nodes.iter().map(|n| n.id).collect();
        net.layout(&all);
        let (a, b) = (net.node(out1).unwrap(), net.node(out2).unwrap());
        assert_eq!(a.x, b.x);
        assert!(a.y < b.y);
        assert!(net.node(col).unwrap().x < a.x);
    }
}
