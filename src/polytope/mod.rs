use bevy::prelude::Mesh;
use bevy::render::mesh::Indices;
use bevy::render::pipeline::PrimitiveTopology;
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, convert::TryInto};

use petgraph::{graph::Graph, prelude::NodeIndex, Undirected};

pub mod convex;
pub mod off;
pub mod shapes;

pub type Element = Vec<usize>;
pub type ElementList = Vec<Element>;
pub type Point = nalgebra::DVector<f64>;
pub type Matrix = nalgebra::DMatrix<f64>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolytopeSerde {
    pub vertices: Vec<Point>,
    pub elements: Vec<ElementList>,
}

impl From<Polytope> for PolytopeSerde {
    fn from(p: Polytope) -> Self {
        PolytopeSerde {
            vertices: p.vertices,
            elements: p.elements,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Polytope {
    /// The concrete vertices of the polytope.
    ///
    /// Here, "dimension" is the number being encoded by the [`Dim`][nalgebra::Dim]
    /// used in [`Point`]s. This is not to be confused by the rank, which is defined
    /// in terms of the length of the element list.
    ///
    /// # Assumptions
    ///
    /// * All points have the same dimension.
    /// * There are neither `NaN` nor `Infinity` values.
    pub vertices: Vec<Point>,

    /// A compact representation of the incidences of the polytope. This vector
    /// stores the edges, faces, ..., of the polytope, all the way up
    /// to the components.
    ///
    /// The (d – 1)-th entry of this vector corresponds to the indices of the
    /// (d – 1)-elements that form a given d-element.
    pub elements: Vec<ElementList>,

    extra_vertices: Vec<Point>,

    triangles: Vec<[usize; 3]>,
}

impl Polytope {
    /// Builds a new [Polytope] with the given vertices and elements.
    pub fn new(vertices: Vec<Point>, elements: Vec<ElementList>) -> Self {
        let (extra_vertices, triangles) = if elements.len() >= 2 {
            Self::triangulate(&vertices, &elements[0], &elements[1])
        } else {
            (vec![], vec![])
        };

        Polytope {
            vertices,
            elements,
            extra_vertices,
            triangles,
        }
    }

    /// Builds a polytope and auto-generates its connected components.
    pub fn new_wo_comps(vertices: Vec<Point>, mut elements: Vec<ElementList>) -> Self {
        let rank = elements.len() + 1;
        assert!(rank >= 2, "new_wo_comps can only work on 2D and higher!");

        let num_ridges = if rank >= 3 {
            elements[rank - 3].len()
        } else {
            vertices.len()
        };

        let facets = &elements[rank - 2];
        let num_facets = facets.len();

        // g is the incidence graph of ridges and facets.
        // The ith ridge is stored at position i.
        // The ith facet is stored at position num_ridges + i.
        let mut graph: Graph<(), (), Undirected> = Graph::new_undirected();

        // Is there not any sort of extend function we can use for syntactic sugar?
        for _ in 0..(num_ridges + num_facets) {
            graph.add_node(());
        }

        // We add an edge for each adjacent facet and ridge.
        for (i, f) in facets.iter().enumerate() {
            for r in f.iter() {
                graph.add_edge(NodeIndex::new(*r), NodeIndex::new(num_ridges + i), ());
            }
        }

        // Converts the connected components of our facet + ridge graph
        // into just the lists of facets in each component.
        let g_comps = petgraph::algo::kosaraju_scc(&graph);
        let mut comps = Vec::with_capacity(g_comps.len());

        for g_comp in g_comps.iter() {
            let mut comp = Vec::new();

            for idx in g_comp.iter() {
                let idx: usize = idx.index();

                if idx < num_ridges {
                    comp.push(idx);
                }
            }

            comps.push(comp);
        }

        elements.push(comps);

        Polytope::new(vertices, elements)
    }

    fn triangulate(
        _vertices: &[Point],
        edges: &[Element],
        faces: &[Element],
    ) -> (Vec<Point>, Vec<[usize; 3]>) {
        let extra_vertices = Vec::new();
        let mut triangles = Vec::new();

        for face in faces {
            let edge_i = *face.first().expect("no indices in face");
            let vert_i = edges
                .get(edge_i)
                .expect("Index out of bounds: you probably screwed up the polytope's indices.")[0];

            for verts in face[1..].iter().map(|&i| {
                edges[i]
                    .clone()
                    .try_into()
                    .expect("edges has more than two elements")
            }) {
                let [vert_j, vert_k]: [usize; 2] = verts;
                if vert_i != vert_j && vert_i != vert_k {
                    triangles.push([vert_i, vert_j, vert_k]);
                }
            }
        }

        (extra_vertices, triangles)
    }

    /// Returns the rank of the polytope.
    pub fn rank(&self) -> usize {
        self.elements.len()
    }

    pub fn dimension(&self) -> usize {
        if self.vertices.is_empty() {
            0
        } else {
            self.vertices[0].len()
        }
    }

    /// Gets the element counts of a polytope.
    /// The n-th entry corresponds to the amount of n-elements.
    pub fn el_nums(&self) -> Vec<usize> {
        let mut nums = Vec::with_capacity(self.elements.len() + 1);
        nums.push(self.vertices.len());

        for e in self.elements.iter() {
            nums.push(e.len());
        }

        nums
    }

    /// Scales a polytope by a given factor.
    pub fn scale(&mut self, k: f64) {
        for v in &mut self.vertices {
            *v *= k;
        }
    }

    pub fn shift(&mut self, o: Point) {
        for v in &mut self.vertices {
            *v -= o.clone();
        }
    }

    pub fn recenter(&mut self) {
        self.shift(self.gravicenter())
    }

    pub fn apply(&mut self, m: &Matrix) {
        for v in &mut self.vertices {
            *v = m.clone() * v.clone();
        }
    }

    fn get_vertex_coords(&self) -> Vec<[f32; 3]> {
        self.vertices
            .iter()
            .chain(self.extra_vertices.iter())
            .map(|point| {
                let mut iter = point.iter().copied().take(3);
                let x = iter.next().unwrap_or(0.0);
                let y = iter.next().unwrap_or(0.0);
                let z = iter.next().unwrap_or(0.0);
                [x as f32, y as f32, z as f32]
            })
            .collect()
    }

    pub fn get_mesh(&self) -> Mesh {
        let vertices = self.get_vertex_coords();
        let mut indices = Vec::with_capacity(self.triangles.len() * 3);
        for &[i, j, k] in &self.triangles {
            indices.push(i as u16);
            indices.push(j as u16);
            indices.push(k as u16);
        }

        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList);
        mesh.set_attribute(
            Mesh::ATTRIBUTE_NORMAL,
            vec![[0.0, 1.0, 0.0]; vertices.len()],
        );
        mesh.set_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; vertices.len()]);
        mesh.set_attribute(Mesh::ATTRIBUTE_POSITION, vertices);
        mesh.set_indices(Some(Indices::U16(indices)));

        mesh
    }

    pub fn get_wireframe(&self) -> Mesh {
        let edges = &self.elements[0];
        let vertices: Vec<_> = self.get_vertex_coords();
        let mut indices = Vec::with_capacity(edges.len() * 2);
        for edge in edges {
            indices.push(edge[0] as u16);
            indices.push(edge[1] as u16);
        }

        let mut mesh = Mesh::new(PrimitiveTopology::LineList);
        mesh.set_attribute(
            Mesh::ATTRIBUTE_NORMAL,
            vec![[0.0, 1.0, 0.0]; vertices.len()],
        );
        mesh.set_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; vertices.len()]);
        mesh.set_attribute(Mesh::ATTRIBUTE_POSITION, vertices);
        mesh.set_indices(Some(Indices::U16(indices)));
        mesh
    }

    pub fn get_element(&self, rank: usize, idx: usize) -> Self {
        struct Sub {
            rank: usize,
            idx: usize,
        }

        let mut sub_indices: Vec<Vec<Option<usize>>> = Vec::with_capacity(rank);
        let mut index_subs: Vec<Vec<usize>> = Vec::with_capacity(rank);

        let vertices = &self.vertices;
        let elements = &self.elements;
        let el_nums = self.el_nums();

        for el_num in el_nums {
            sub_indices.push(vec![None; el_num]);
            index_subs.push(vec![]);
        }

        let mut sub_deque = VecDeque::new();
        sub_deque.push_back(Sub { rank, idx });

        let mut c = vec![0; rank];
        while let Some(sub) = sub_deque.pop_front() {
            let d = sub.rank - 1;
            let i = sub.idx;

            let els = &elements[d];

            for &j in &els[i] {
                if sub_indices[d][j] == None {
                    sub_indices[d][j] = Some(c[d]);
                    index_subs[d].push(j);
                    c[d] += 1;

                    if d > 0 {
                        sub_deque.push_back(Sub { rank: d, idx: j });
                    }
                }
            }
        }

        let mut new_vertices = Vec::with_capacity(index_subs[0].len());
        for &i in &index_subs[0] {
            new_vertices.push(vertices[i].clone());
        }

        let mut new_elements = Vec::with_capacity(rank);
        for d in 1..rank {
            new_elements.push(Vec::with_capacity(index_subs[d].len()));
            for &i in &index_subs[d] {
                let mut el = elements[d - 1][i].clone();

                for sub in &mut el {
                    *sub = sub_indices[d - 1][*sub].unwrap();
                }

                new_elements[d - 1].push(el);
            }
        }

        let facets = elements[rank - 1][idx].len();
        let mut components = vec![Vec::with_capacity(facets)];
        for i in 0..facets {
            components[0].push(i);
        }
        new_elements.push(components);

        Polytope::new(new_vertices, new_elements)
    }

    pub fn verf(&self, idx: usize) -> Polytope {
        let dual = shapes::dual(self);
        let facet = dual.get_element(self.rank() - 1, idx);

        shapes::dual(&facet)
    }

    pub fn gravicenter(&self) -> Point {
        let dim = self.dimension();
        let mut g: Point = vec![0.0; dim].into();
        let vertices = &self.vertices;

        for v in vertices {
            g += v;
        }

        g / (vertices.len() as f64)
    }
}

impl From<PolytopeSerde> for Polytope {
    fn from(ps: PolytopeSerde) -> Self {
        Polytope::new(ps.vertices, ps.elements)
    }
}
