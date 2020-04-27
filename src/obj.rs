//   Copyright 2017 GFX Developers
//
//   Licensed under the Apache License, Version 2.0 (the "License");
//   you may not use this file except in compliance with the License.
//   You may obtain a copy of the License at
//
//       http://www.apache.org/licenses/LICENSE-2.0
//
//   Unless required by applicable law or agreed to in writing, software
//   distributed under the License is distributed on an "AS IS" BASIS,
//   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//   See the License for the specific language governing permissions and
//   limitations under the License.

//! Parsing and writing of a .obj file as defined in the
//! [full spec](http://paulbourke.net/dataformats/obj/).

#[cfg(feature = "genmesh")]
pub use genmesh::{Polygon, Quad, Triangle};

use std::sync::Arc;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, BufRead, BufReader, Error, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::fmt;

use mtl::{Material, Mtl, MtlError};

const DEFAULT_OBJECT: &str = "default";
const DEFAULT_GROUP: &str = "default";

#[derive(Debug, Clone, Copy, Hash, PartialEq, PartialOrd, Eq, Ord)]
pub struct IndexTuple(pub usize, pub Option<usize>, pub Option<usize>);
pub type SimplePolygon = Vec<IndexTuple>;

pub trait WriteToBuf {
    type Error: std::fmt::Display;
    fn write_to_buf<W: Write>(&self, out: &mut W) -> Result<(), Self::Error>;
}

pub trait GenPolygon: Clone {
    fn new(line_number: usize, data: SimplePolygon) -> Self;
    fn try_new(line_number: usize, data: SimplePolygon) -> Result<Self, ObjError>;
}

impl std::fmt::Display for IndexTuple {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0 + 1)?;
        if let Some(idx) = self.1 {
            write!(f, "/{}", idx + 1)?;
        }
        if let Some(idx) = self.2 {
            write!(f, "/{}", idx + 1)?;
        }
        Ok(())
    }
}

impl GenPolygon for SimplePolygon {
    fn new(_line_number: usize, data: Self) -> Self {
        data
    }
    fn try_new(_line_number: usize, data: SimplePolygon) -> Result<Self,ObjError> {
        Ok(data)
    }
}

impl WriteToBuf for SimplePolygon {
    type Error = ObjError;
    fn write_to_buf<W: Write>(&self, out: &mut W) -> Result<(), ObjError> {
        write!(out, "f")?;
        for idx in self {
            write!(out, " {}", idx)?;
        }
        writeln!(out)?;
        Ok(())
    }
}


#[cfg(feature = "genmesh")]
impl WriteToBuf for Polygon<IndexTuple> {
    type Error = ObjError;
    fn write_to_buf<W: Write>(&self, out: &mut W) -> Result<(), ObjError> {
        match self {
            Polygon::PolyTri(tri) => write!(out, "f {} {} {}", tri.x, tri.y, tri.z)?,
            Polygon::PolyQuad(quad) => write!(out, "f {} {} {}", quad.x, quad.y, quad.z)?,
        }
        writeln!(out)
    }
}

#[cfg(feature = "genmesh")]
impl GenPolygon for Polygon<IndexTuple> {
    fn new(line_number: usize, gs: SimplePolygon) -> Self {
        Polygon::<IndexTuple>::try_new(line_number, gs).unwrap()
    }
    fn try_new(line_number: usize, gs: SimplePolygon) -> Result<Self,ObjError> {
        match gs.len() {
            3 => Ok(Polygon::PolyTri(Triangle::new(gs[0], gs[1], gs[2]))),
            4 => Ok(Polygon::PolyQuad(Quad::new(gs[0], gs[1], gs[2], gs[3]))),
            n => return Err(ObjError::GenMeshTooManyVertsInPolygon {line_number, vert_count: n}),
        }
    }
}

/// Errors parsing or loading a .obj file.
#[derive(Debug)]
pub enum ObjError {
    Io(io::Error),
    /// One of the arguments to `f` is malformed.
    MalformedFaceGroup {
        line_number: usize,
        group: String,
    },
    /// An argument list either has unparsable arguments or is
    /// missing one or more arguments.
    ArgumentListFailure {
        line_number: usize,
        list: String,
    },
    /// Command found that is not in the .obj spec.
    UnexpectedCommand {
        line_number: usize,
        command: String,
    },
    /// `mtllib` command issued, but no name was specified.
    MissingMTLName {
        line_number: usize,
    },
    /// [`genmesh::Polygon`] only supports triangles and squares.
    #[cfg(feature = "genmesh")]
    GenMeshTooManyVertsInPolygon {
        line_number: usize,
        vert_count: usize,
    },
}

impl std::error::Error for ObjError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ObjError::Io(err) => Some(err),
            _ => None
        }
    }
}

impl fmt::Display for ObjError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObjError::Io(err) => write!(f, "I/O error loading a .obj file: {}", err),
            ObjError::MalformedFaceGroup { line_number, group, } =>
                write!(f, "One of the arguments to `f` is malformed (line: {}, group: {})", line_number, group),
            ObjError::ArgumentListFailure { line_number, list } =>
                write!(f, "An argument list either has unparsable arguments or is missing arguments. (line: {}, list: {})" ,
                        line_number, list),
            ObjError::UnexpectedCommand { line_number, command } =>
                write!(f, "Command found that is not in the .obj spec. (line: {}, command: {})", line_number, command),
            ObjError::MissingMTLName { line_number } =>
                write!(f, "mtllib command issued, but no name was specified. (line: {})", line_number),
            #[cfg(feature = "genmesh")]
            ObjError::GenMeshTooManyVertsInPolygon { line_number, vert_count } =>
                write!(f, "[`genmesh::Polygon`] only supports triangles and squares. (line: {}, vertex count: {}", line_number, vert_count),
        }
    }
}

impl From<io::Error> for ObjError {
    fn from(e: Error) -> Self {
        Self::Io(e)
    }
}


/// Error loading individual material libraries.
///
/// The `Vec` items are tuples with first component being the the .mtl file, and the second its
/// corresponding error.
#[derive(Debug)]
pub struct MtlLibsLoadError(Vec<(String, MtlError)>);

impl std::error::Error for MtlLibsLoadError { }

impl fmt::Display for MtlLibsLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "One of the material libraries failed to load: {:?}", self.0)
    }
}

impl From<Vec<(String, MtlError)>> for MtlLibsLoadError {
    fn from(e: Vec<(String, MtlError)>) -> Self {
        MtlLibsLoadError(e)
    }
}


#[derive(Debug, Clone, PartialEq)]
pub struct Object<P = SimplePolygon> {
    pub name: String,
    pub groups: Vec<Group<P>>,
}

impl<P> Object<P> {
    pub fn new(name: String) -> Self {
        Object { name: name, groups: Vec::new() }
    }
}


impl<P: WriteToBuf<Error = ObjError>> WriteToBuf for Object<P> {
    type Error = ObjError;
    /// Serialize this `Object` into the given writer.
    fn write_to_buf<W: Write>(&self, out: &mut W) -> Result<(), ObjError> {
        if self.name.as_str() != DEFAULT_OBJECT {
            writeln!(out, "o {}", self.name)?;
        }

        let mut group_iter = self.groups.iter().peekable();
        while let Some(group) = group_iter.next() {
            group.write_to_buf(out)?;

            // Below we check that groups with `index > 0` have the same name as their predecessors
            // which enables us to merge the two by omitting the additional `g ...` command.
            assert!(group_iter.peek().map(|next_group| next_group.index == 0 || next_group.name == group.name).unwrap_or(true));
        }

        Ok(())
    }
}

/// The data represented by the `usemtl` command.
///
/// The material name is replaced by the actual material data when the material libraries are
/// laoded if a match is found.
#[derive(Debug, Clone, PartialEq)]
pub enum ObjMaterial {
    /// A reference to a material as a material name.
    Ref(String),
    /// A complete `Material` object loaded from a .mtl file in place of the material reference.
    Mtl(Arc<Material>),
}

impl ObjMaterial {
    fn name(&self) -> &str {
        match self {
            ObjMaterial::Ref(name) => name.as_str(),
            ObjMaterial::Mtl(material) => material.name.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group<P = SimplePolygon> {
    pub name: String,
    /// An index is used to tell groups apart that share the same name
    pub index: usize,
    pub material: Option<ObjMaterial>,
    pub polys: Vec<P>,
}

impl<P> Group<P> {
    pub fn new(name: String) -> Self {
        Group {
            name: name,
            index: 0,
            material: None,
            polys: Vec::new(),
        }
    }
}

impl<P: WriteToBuf<Error = ObjError>> WriteToBuf for Group<P> {
    type Error = ObjError;
    /// Serialize this `Group` into the given writer.
    fn write_to_buf<W: Write>(&self, out: &mut W) -> Result<(), ObjError> {
        // When index is greater than 0, we know that this group is the same as the previous group,
        // so don't bother declaring a new one.
        if self.index == 0 {
            writeln!(out, "g {}", self.name)?;
        }

        match self.material {
            Some(ObjMaterial::Ref(ref name)) => writeln!(out, "usemtl {}", name)?,
            Some(ObjMaterial::Mtl(ref mtl)) => writeln!(out, "usemtl {}", mtl.name)?,
            None => {}
        }

        for poly in &self.polys {
            poly.write_to_buf(out)?;
        }

        Ok(())
    }
}

/// The data model associated with each `Obj` file.
#[derive(Debug, PartialEq)]
pub struct ObjData<P = SimplePolygon> {
    /// Vertex positions.
    pub position: Vec<[f32; 3]>,
    /// 2D texture coordinates.
    pub texture: Vec<[f32; 2]>,
    /// A set of normals.
    pub normal: Vec<[f32; 3]>,
    /// A collection of associated objects indicated by `o`, as well as the default object at the
    /// top level.
    pub objects: Vec<Object<P>>,
    /// The set of all `mtllib` references to .mtl files.
    pub material_libs: Vec<Mtl>,
}

impl<P> Default for ObjData<P> {
    fn default() -> Self {
        ObjData {
            position: Vec::new(),
            texture: Vec::new(),
            normal: Vec::new(),
            objects: Vec::new(),
            material_libs: Vec::new(),
        }
    }
}


/// A struct used to store `Obj` data as well as its source directory used to load the referenced
/// .mtl files.
#[derive(Debug)]
pub struct Obj<P = SimplePolygon> {
    /// The data associated with this `Obj` file.
    pub data: ObjData<P>,
    /// The path of the parent directory from which this file was read.
    ///
    /// It is not always set since the file may have been read from a `String`.
    pub path: PathBuf,
}

fn normalize(idx: isize, len: usize) -> usize {
    if idx < 0 {
        (len as isize + idx) as usize
    } else {
        idx as usize - 1
    }
}

impl<P: WriteToBuf<Error = ObjError>> Obj<P> {
    /// Save the current `Obj` at the given file path as well as any associated .mtl files.
    ///
    /// If a file already exists, it will be overwritten.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ObjError> {
        self.data.save(path.as_ref())
    }
}

impl<P: GenPolygon> Obj<P> {
    /// Load an `Obj` file from the given path.
    pub fn load(path: impl AsRef<Path>) -> Result<Obj<P>, ObjError> {
        Obj::load_impl(path.as_ref())
    }

    fn load_impl(path: &Path) -> Result<Obj<P>, ObjError> {
        let f = File::open(path)?;
        let data = ObjData::load_buf(&f)?;

        // unwrap is safe since we've read this file before.
        let path = path.parent().unwrap().to_owned();

        Ok(Obj { data, path })
    }

    /// Loads the .mtl files referenced in the .obj file.
    ///
    /// If it encounters an error for an .mtl, it appends its error to the
    /// returning Vec, and tries the rest.
    pub fn load_mtls(&mut self) -> Result<(), MtlLibsLoadError> {
        self.load_mtls_fn(|obj_dir, mtllib| File::open(&obj_dir.join(mtllib)).map(BufReader::new))
    }

    /// Loads the .mtl files referenced in the .obj file with user provided loading logic.
    ///
    /// See also [`load_mtls`].
    ///
    /// The provided function must take two arguments:
    ///  - `&Path` - The parent directory of the .obj file
    ///  - `&str`  - The name of the mtllib as listed in the file.
    ///
    /// This function allows loading .mtl files in directories different from the default .obj
    /// directory.
    ///
    /// It must return:
    ///  - Anything that implements [`io::BufRead`] that yields the contents of the intended .mtl file.
    ///
    /// [`load_mtls`]: #method.load_mtls
    /// [`io::BufRead`]: https://doc.rust-lang.org/std/io/trait.BufRead.html
    pub fn load_mtls_fn<R, F>(&mut self, mut resolve: F) -> Result<(), MtlLibsLoadError>
        where
            R: io::BufRead,
            F: FnMut(&Path, &str) -> io::Result<R> {
        let mut errs = Vec::new();
        let mut materials = HashMap::new();

        for mtl_lib in &mut self.data.material_libs {
            match mtl_lib.reload_with(&self.path, &mut resolve) {
                Ok(mtl_lib) => {
                    for m in &mtl_lib.materials {
                        // We don't want to overwrite existing entries because of how the materials
                        // are looked up. From the spec:
                        // "If multiple filenames are specified, the first file
                        //  listed is searched first for the material definition, the second
                        //  file is searched next, and so on."
                        materials.entry(m.name.clone()).or_insert(Arc::clone(m));
                    }
                },
                Err(err) => {
                    errs.push((mtl_lib.filename.clone(), err));
                },
            }
        }

        // Assign loaded materials to the corresponding objects.
        for object in &mut self.data.objects {
            for group in &mut object.groups {
                if let Some(ref mut mat) = group.material {
                    if let Some(newmat) = materials.get(mat.name()) {
                        *mat = ObjMaterial::Mtl(Arc::clone(newmat));
                    }
                }
            }
        }

        if errs.is_empty() { Ok(()) } else { Err(errs.into()) }
    }
}

impl<P: WriteToBuf<Error = ObjError>> ObjData<P> {
    /// Save the current `ObjData` at the given file path as well as any associated .mtl files.
    ///
    /// If a file already exists, it will be overwritten.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ObjError> {
        self.save_impl(path.as_ref())
    }

    fn save_impl(&self, path: &Path) -> Result<(), ObjError> {
        let mut f = File::create(path)?;
        self.write_to_buf(&mut f)?;

        // unwrap is safe because we created the file above.
        let path = path.parent().unwrap();
        self.save_mtls(path)
    }

    /// Save all material libraries referenced in this `Obj` to the given base directory.
    pub fn save_mtls(&self, base_dir: impl AsRef<Path>) -> Result<(), ObjError> {
        self.save_mtls_with_fn(base_dir.as_ref(), |base_dir, mtllib| File::create(base_dir.join(mtllib)))
    }

    /// Save all material libraries referenced in this `Obj` struct according to `resolve`.
    pub fn save_mtls_with_fn<W: Write>(&self, base_dir: &Path, mut resolve: impl FnMut(&Path, &str) -> io::Result<W>) -> Result<(), ObjError> {
        for mtl in &self.material_libs {
            mtl.write_to_buf(&mut resolve(base_dir, &mtl.filename)?)?;
        }
        Ok(())
    }

    /// Serialize this `Obj` into the given writer.
    pub fn write_to_buf(&self, out: &mut impl Write) -> Result<(), ObjError> {
        writeln!(out, "# Generated by the obj Rust library (https://crates.io/crates/obj).")?;

        for pos in &self.position {
            writeln!(out, "v {} {} {}", pos[0], pos[1], pos[2])?;
        }
        for uv in &self.texture {
            writeln!(out, "vt {} {}", uv[0], uv[1])?;
        }
        for nml in &self.normal {
            writeln!(out, "vn {} {} {}", nml[0], nml[1], nml[2])?;
        }
        for object in &self.objects {
            object.write_to_buf(out)?;
        }
        for mtl_lib in &self.material_libs {
            writeln!(out, "mtllib {}", mtl_lib.filename)?;
        }

        Ok(())
    }
}

impl<P: GenPolygon> ObjData<P> {
    fn parse_two(line_number: usize, n0: Option<&str>, n1: Option<&str>) -> Result<[f32; 2], ObjError> {
        let (n0, n1) = match (n0, n1) {
            (Some(n0), Some(n1)) => (n0, n1),
            _ => {
                return Err(ObjError::ArgumentListFailure { line_number, list: format!("{:?} {:?}", n0, n1)});
            }
        };
        let normal = match (FromStr::from_str(n0), FromStr::from_str(n1)) {
            (Ok(n0), Ok(n1)) => [n0, n1],
            _ => {
                return Err(ObjError::ArgumentListFailure { line_number, list: format!("{:?} {:?}", n0, n1)});
            }
        };
        Ok(normal)
    }

    fn parse_three(line_number: usize, n0: Option<&str>, n1: Option<&str>, n2: Option<&str>) -> Result<[f32; 3], ObjError> {
        let (n0, n1, n2) = match (n0, n1, n2) {
            (Some(n0), Some(n1), Some(n2)) => (n0, n1, n2),
            _ => {
                return Err(ObjError::ArgumentListFailure { line_number, list: format!("{:?} {:?} {:?}", n0, n1, n2)});
            }
        };
        let normal = match (FromStr::from_str(n0), FromStr::from_str(n1), FromStr::from_str(n2)) {
            (Ok(n0), Ok(n1), Ok(n2)) => [n0, n1, n2],
            _ => {
                return Err(ObjError::ArgumentListFailure { line_number, list: format!("{:?} {:?} {:?}", n0, n1, n2)});
            }
        };
        Ok(normal)
    }

    fn parse_group(&self, line_number: usize, group: &str) -> Result<IndexTuple, ObjError> {
        let mut group_split = group.split('/');
        let p: Option<isize> = group_split.next().and_then(|idx| FromStr::from_str(idx).ok());
        let t: Option<isize> =
            group_split.next().and_then(|idx| if idx != "" { FromStr::from_str(idx).ok() } else { None });
        let n: Option<isize> = group_split.next().and_then(|idx| FromStr::from_str(idx).ok());

        match (p, t, n) {
            (Some(p), t, n) => {
                Ok(IndexTuple(normalize(p, self.position.len()),
                              t.map(|t| normalize(t, self.texture.len())),
                              n.map(|n| normalize(n, self.normal.len()))))
            }
            _ => Err(ObjError::MalformedFaceGroup {line_number, group: String::from(group)}),
        }
    }

    fn parse_face<'b, I>(&self, line_number: usize, groups: &mut I) -> Result<P, ObjError>
    where
        I: Iterator<Item = &'b str>,
    {
        let mut ret = Vec::with_capacity(4);
        for g in groups {
            let ituple = self.parse_group(line_number, g)?;
            ret.push(ituple);
        }
        P::try_new(line_number, ret)
    }

    pub fn load_buf<R: Read>(input: R) -> Result<Self, ObjError> {
        let input = BufReader::new(input);
        let mut dat = ObjData::default();
        let mut object = Object::new(DEFAULT_OBJECT.to_string());
        let mut group: Option<Group<P>> = None;

        for (idx, line) in input.lines().enumerate() {
            let (line, mut words) = match line {
                Ok(ref line) => (line.clone(), line.split_whitespace().filter(|s| !s.is_empty())),
                Err(err) => {
                    return Err(ObjError::Io(io::Error::new(io::ErrorKind::InvalidData, format!("failed to readline {}", err))));
                }
            };
            let first = words.next();

            match first {
                Some("v") => {
                    let (v0, v1, v2) = (words.next(), words.next(), words.next());
                    dat.position.push(Self::parse_three(idx, v0, v1, v2)?);
                }
                Some("vt") => {
                    let (t0, t1) = (words.next(), words.next());
                    dat.texture.push(Self::parse_two(idx, t0, t1)?);
                }
                Some("vn") => {
                    let (n0, n1, n2) = (words.next(), words.next(), words.next());
                    dat.normal.push(Self::parse_three(idx, n0, n1, n2)?);
                }
                Some("f") => {
                    let poly = dat.parse_face(idx, &mut words)?;
                    group = Some(match group {
                                     None => {
                                         let mut g = Group::new(DEFAULT_GROUP.to_string());
                                         g.polys.push(poly);
                                         g
                                     }
                                     Some(mut g) => {
                                         g.polys.push(poly);
                                         g
                                     }
                                 });
                }
                Some("o") => {
                    group = match group {
                        Some(val) => {
                            object.groups.push(val);
                            dat.objects.push(object);
                            None
                        }
                        None => None,
                    };
                    object = if line.len() > 2 {
                        let name = line[1..].trim();
                        Object::new(name.to_string())
                    } else {
                        Object::new(DEFAULT_OBJECT.to_string())
                    };
                }
                Some("g") => {
                    object.groups.extend(group.take());

                    if line.len() > 2 {
                        let name = line[2..].trim();
                        group = Some(Group::new(name.to_string()));
                    }
                }
                Some("mtllib") => {
                    // Obj strictly does not allow spaces in filenames.
                    // "mtllib Some File.mtl" is forbidden.
                    // However, everyone does it anyway and if we want to ingest blender-outputted files, we need to support it.
                    // This works by walking word by word and combining them with a space in between. This may not be a totally
                    // accurate way to do it, but until the parser can be re-worked, this is good-enough, better-than-before solution.
                    let first_word = words.next().ok_or_else(|| ObjError::MissingMTLName {line_number: idx})?.to_string();
                    let name = words.fold(first_word, |mut existing, next| {
                        existing.push(' ');
                        existing.push_str(next);
                        existing
                    });
                    dat.material_libs.push(Mtl::new(name));
                }
                Some("usemtl") => {
                    let mut g = match group {
                        Some(g) => g,
                        None => Group::new(DEFAULT_GROUP.to_string()),
                    };
                    // we found a new material that was applied to an existing
                    // object. It is treated as a new group.
                    if g.material.is_some() {
                        object.groups.push(g.clone());
                        g.index += 1;
                        g.polys.clear();
                    }
                    g.material = match words.next() {
                        Some(w) => Some(ObjMaterial::Ref(w.to_string())),
                        None => None,
                    };
                    group = Some(g);
                }
                Some("s") => (),
                Some("l") => (),
                Some(other) => {
                    if !other.starts_with("#") {
                        return Err(ObjError::UnexpectedCommand {line_number: idx, command: other.to_string()})
                    }
                }
                None => (),
            }
        }
        match group {
            Some(g) => object.groups.push(g),
            None => (),
        };
        dat.objects.push(object);
        Ok(dat)
    }
}
