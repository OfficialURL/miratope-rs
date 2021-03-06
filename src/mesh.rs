//! Contains the methods that take a polytope and turn it into a mesh.

use std::collections::HashMap;

use crate::ui::camera::ProjectionType;

use bevy::{
    prelude::Mesh,
    render::{mesh::Indices, pipeline::PrimitiveTopology},
};
use lyon::{math::point, path::Path, tessellation::*};
use miratope_core::{
    abs::{elements::ElementList, rank::Rank},
    conc::{
        cycle::{Cycle, CycleBuilder},
        Concrete, ConcretePolytope,
    },
    geometry::{Point, Subspace, Vector},
    Consts, Float, Polytope,
};

use vec_like::*;

/// Attempts to turn the cycle into a 2D path, which can then be given to
/// the tessellator. Uses the specified vertex list to grab the coordinates
/// of the vertices on the path.
///
/// If the cycle isn't 2D, we return `None`.
pub fn path(cycles: &[Cycle], vertices: &[Point]) -> Option<Path> {
    let dim = vertices[0].len();
    let mut builder = Path::builder();

    for (idx, cycle) in cycles.iter().enumerate() {
        let mut cycle_iter = cycle.iter().map(|&idx| &vertices[idx]);

        // We don't bother with any polygons that aren't in 2D space.
        let s = Subspace::from_points_with(cycle_iter.clone(), 2)?;

        // We find the two axis directions most convenient for projecting down.
        // Convenience is measured as the length of an axis vector projected
        // down onto the plane our cycle lies in.

        // The index of the axis vector that gives the largest length when
        // projected, and that such length.
        let mut idx0 = 0;
        let mut len0 = 0.0;

        // The index of the axis vector that gives the second largest length
        // when projected, and that such length.
        let mut idx1 = 0;
        let mut len1 = 0.0;

        // We compute idx0 and idx1 real quick.
        let mut e = Point::zeros(dim);
        for i in 0..dim {
            e[i] = 1.0;

            let len = s.project(&e).norm();
            // This is the largest length we've found so far.
            if len > len0 {
                len1 = len0;
                idx1 = idx0;
                len0 = len;
                idx0 = i;
            }
            // This is the second largest length we've found so far.
            else if len > len1 {
                len1 = len;
                idx1 = i;
            }

            e[i] = 0.0;
        }

        // Converts a point in the polytope to a point in the path via
        // orthogonal projection at our convenient axes.
        let path_point = |v: &Point| point(v[idx0] as f32, v[idx1] as f32);

        // We build a path from the polygon.
        let v = cycle_iter.next().unwrap();
        builder.begin(path_point(v));

        for v in cycle_iter {
            builder.line_to(path_point(v));
        }

        builder.end(idx + 1 == cycles.len());
    }

    Some(builder.build())
}

/// Represents a triangulation of the faces of a [`Concrete`]. It stores the
/// vertex indices that make up the triangulation of the polytope, as well as
/// the extra vertices that may be needed to represent it.
struct Triangulation {
    /// Extra vertices that might be needed for the triangulation.
    extra_vertices: Vec<Point>,

    /// Indices of the vertices that make up the triangles.
    triangles: Vec<u16>,
}

impl Triangulation {
    /// Creates a new triangulation from a polytope.
    fn new(polytope: &Concrete) -> Triangulation {
        let mut extra_vertices = Vec::new();
        let mut triangles = Vec::new();

        let empty_els = ElementList::new();

        // Either returns a reference to the element list of a given rank, or
        // returns a reference to an empty element list.
        let elements_or = |r| polytope.abs.ranks.get(r).unwrap_or(&empty_els);

        let edges = elements_or(Rank::new(1));
        let faces = elements_or(Rank::new(2));

        let concrete_vertex_len = polytope.vertices.len() as u16;

        // We render each face separately.
        for face in faces {
            let mut vertex_loop = CycleBuilder::with_capacity(face.subs.len());

            // We first figure out the vertices in order.
            for [v0, v1] in face.subs.iter().map(|&i| {
                let subs = &edges[i].subs;
                let len = subs.len();

                debug_assert_eq!(len, 2, "Edge has {} subelements, expected 2.", len);
                [subs[0], subs[1]]
            }) {
                vertex_loop.push(v0, v1);
            }

            // We tesselate this path.
            let cycles = vertex_loop.cycles();
            if let Some(path) = path(&cycles, &polytope.vertices) {
                let mut geometry: VertexBuffers<_, u16> = VertexBuffers::new();

                // Configures all of the options of the tessellator.
                FillTessellator::new()
                    .tessellate_with_ids(
                        path.id_iter(),
                        &path,
                        None,
                        &FillOptions::with_fill_rule(Default::default(), FillRule::EvenOdd)
                            .with_tolerance(f32::EPS),
                        &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex| {
                            vertex.sources().next().unwrap()
                        }),
                    )
                    .unwrap();

                // Maps EndpointIds to the indices in the original vertex list.
                let mut id_to_idx = Vec::new();
                for cycle in cycles {
                    for idx in cycle {
                        id_to_idx.push(idx);
                    }
                }

                // We map the output vertices to the original ones, and add any
                // extra vertices that may be needed.
                let mut vertex_hash = HashMap::new();

                for (new_id, vertex_source) in geometry.vertices.into_iter().enumerate() {
                    let new_id = new_id as u16;

                    match vertex_source {
                        // This is one of the concrete vertices of the polytope.
                        VertexSource::Endpoint { id } => {
                            vertex_hash.insert(new_id, id_to_idx[id.to_usize()] as u16);
                        }

                        // This is a new vertex that has been added to the tesselation.
                        VertexSource::Edge { from, to, t } => {
                            let from = &polytope.vertices[id_to_idx[from.to_usize()]];
                            let to = &polytope.vertices[id_to_idx[to.to_usize()]];

                            let t = t as Float;
                            let p = from * (1.0 - t) + to * t;

                            vertex_hash
                                .insert(new_id, concrete_vertex_len + extra_vertices.len() as u16);

                            extra_vertices.push(p);
                        }
                    }
                }

                // Add all of the new indices we've found onto the triangle vector.
                for new_idx in geometry
                    .indices
                    .iter()
                    .map(|idx| *vertex_hash.get(idx).unwrap())
                {
                    triangles.push(new_idx);
                }
            }
        }

        Self {
            extra_vertices,
            triangles,
        }
    }
}

/// Generates normals from a set of vertices by just projecting radially from
/// the origin.
fn normals(vertices: &[[f32; 3]]) -> Vec<[f32; 3]> {
    vertices
        .iter()
        .map(|n| {
            let sq_norm = n[0] * n[0] + n[1] * n[1] + n[2] * n[2];
            if sq_norm < f32::EPS {
                [0.0, 0.0, 0.0]
            } else {
                let norm = sq_norm.sqrt();
                let mut n = *n;
                n[0] /= norm;
                n[1] /= norm;
                n[2] /= norm;
                n
            }
        })
        .collect()
}

/// Returns an empty mesh.
fn empty_mesh() -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::LineList);
    mesh.set_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0; 3]]);
    mesh.set_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0; 3]]);
    mesh.set_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0; 2]]);
    mesh.set_indices(Some(Indices::U16(Vec::new())));

    mesh
}

/// Gets the coordinates of the vertices, after projecting down into 3D.
fn vertex_coords<'a, T: Iterator<Item = &'a Point>>(
    poly: &Concrete,
    vertices: T,
    projection_type: ProjectionType,
) -> Vec<[f32; 3]> {
    let dim = poly.dim_or();

    // If the polytope is at most 3D, we just embed it into 3D space.
    if projection_type.is_orthogonal() || dim <= 3 {
        vertices
            .map(|point| {
                let mut iter = point.iter().take(3).map(|&c| c as f32);
                let x = iter.next().unwrap_or(0.0);
                let y = iter.next().unwrap_or(0.0);
                let z = iter.next().unwrap_or(0.0);
                [x, y, z]
            })
            .collect()
    }
    // Else, we project it down.
    else {
        // Distance from the projection planes.
        let mut direction = Vector::zeros(dim);
        direction[3] = 1.0;

        let (min, max) = poly.minmax(&direction).unwrap();
        let dist = (min as f32 - 1.0).abs().max(max as f32 + 1.0).abs();

        vertices
            .map(|point| {
                let factor: f32 = point.iter().skip(3).map(|&x| x as f32 + dist).product();

                // We scale the first three coordinates accordingly.
                let mut iter = point.iter().copied().take(3).map(|c| c as f32 / factor);
                let x = iter.next().unwrap();
                let y = iter.next().unwrap();
                let z = iter.next().unwrap();
                [x, y, z]
            })
            .collect()
    }
}

/// Builds the mesh of a polytope.
pub fn mesh(poly: &Concrete, projection_type: ProjectionType) -> Mesh {
    // If there's no vertices, returns an empty mesh.
    if poly.vertex_count() == 0 {
        return empty_mesh();
    }

    // Triangulates the polytope's faces, projects the vertices of both the
    // polytope and the triangulation.
    let triangulation = Triangulation::new(poly);
    let vertices = vertex_coords(
        &poly,
        poly.vertices
            .iter()
            .chain(triangulation.extra_vertices.iter()),
        projection_type,
    );

    // Builds the actual mesh.
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList);
    mesh.set_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 1.0]; vertices.len()]);
    mesh.set_attribute(Mesh::ATTRIBUTE_NORMAL, normals(&vertices));
    mesh.set_attribute(Mesh::ATTRIBUTE_POSITION, vertices);
    mesh.set_indices(Some(Indices::U16(triangulation.triangles)));

    mesh
}

/// Builds the wireframe of a polytope.
pub fn wireframe(poly: &Concrete, projection_type: ProjectionType) -> Mesh {
    let vertex_count = poly.vertex_count();

    // If there's no vertices, returns an empty mesh.
    if vertex_count == 0 {
        return empty_mesh();
    }

    let edges = poly.abs.ranks.get(Rank::new(1));
    let edge_count = poly.el_count(Rank::new(1));

    // We add a single vertex so that Miratope doesn't crash.
    let vertices = vertex_coords(&poly, poly.vertices.iter(), projection_type);
    let mut indices = Vec::with_capacity(edge_count * 2);

    // Adds the edges to the wireframe.
    if let Some(edges) = edges {
        for edge in edges {
            debug_assert_eq!(
                edge.subs.len(),
                2,
                "Edge must have exactly 2 elements, found {}.",
                edge.subs.len()
            );

            indices.push(edge.subs[0] as u16);
            indices.push(edge.subs[1] as u16);
        }
    }

    // Sets the mesh attributes.
    let mut mesh = Mesh::new(PrimitiveTopology::LineList);
    mesh.set_attribute(Mesh::ATTRIBUTE_NORMAL, normals(&vertices));
    mesh.set_attribute(Mesh::ATTRIBUTE_POSITION, vertices);
    mesh.set_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0; 2]; vertex_count]);
    mesh.set_indices(Some(Indices::U16(indices)));

    mesh
}
