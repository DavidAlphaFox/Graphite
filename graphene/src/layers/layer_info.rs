use super::blend_mode::BlendMode;
use super::folder::Folder;
use super::simple_shape::Shape;
use super::style::{PathStyle, ViewMode};
use super::text::Text;
use crate::intersection::Quad;
use crate::DocumentError;
use crate::LayerId;

use glam::{DAffine2, DMat2, DVec2, Vec2};
use kurbo::PathEl;
use serde::{Deserialize, Serialize};
use std::fmt::Write;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub enum LayerDataType {
	Folder(Folder),
	Shape(Shape),
	Text(Text),
}

impl LayerDataType {
	pub fn inner(&self) -> &dyn LayerData {
		match self {
			LayerDataType::Shape(s) => s,
			LayerDataType::Folder(f) => f,
			LayerDataType::Text(t) => t,
		}
	}

	pub fn inner_mut(&mut self) -> &mut dyn LayerData {
		match self {
			LayerDataType::Shape(s) => s,
			LayerDataType::Folder(f) => f,
			LayerDataType::Text(t) => t,
		}
	}
}

pub trait LayerData {
	fn render(&mut self, svg: &mut String, transforms: &mut Vec<glam::DAffine2>, view_mode: ViewMode);
	fn intersects_quad(&self, quad: Quad, path: &mut Vec<LayerId>, intersections: &mut Vec<Vec<LayerId>>);
	fn bounding_box(&self, transform: glam::DAffine2) -> Option<[DVec2; 2]>;
}

impl LayerData for LayerDataType {
	fn render(&mut self, svg: &mut String, transforms: &mut Vec<glam::DAffine2>, view_mode: ViewMode) {
		self.inner_mut().render(svg, transforms, view_mode)
	}

	fn intersects_quad(&self, quad: Quad, path: &mut Vec<LayerId>, intersections: &mut Vec<Vec<LayerId>>) {
		self.inner().intersects_quad(quad, path, intersections)
	}

	fn bounding_box(&self, transform: glam::DAffine2) -> Option<[DVec2; 2]> {
		self.inner().bounding_box(transform)
	}
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "glam::DAffine2")]
struct DAffine2Ref {
	pub matrix2: DMat2,
	pub translation: DVec2,
}

fn return_true() -> bool {
	true
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Layer {
	pub visible: bool,
	pub name: Option<String>,
	pub data: LayerDataType,
	#[serde(with = "DAffine2Ref")]
	pub transform: glam::DAffine2,
	#[serde(skip)]
	pub cache: String,
	#[serde(skip)]
	pub thumbnail_cache: String,
	#[serde(skip, default = "return_true")]
	pub cache_dirty: bool,
	pub blend_mode: BlendMode,
	pub opacity: f64,
}

impl Layer {
	pub fn new(data: LayerDataType, transform: [f64; 6]) -> Self {
		Self {
			visible: true,
			name: None,
			data,
			transform: glam::DAffine2::from_cols_array(&transform),
			cache: String::new(),
			thumbnail_cache: String::new(),
			cache_dirty: true,
			blend_mode: BlendMode::Normal,
			opacity: 1.,
		}
	}

	pub fn iter(&self) -> LayerIter<'_> {
		LayerIter { stack: vec![self] }
	}

	pub fn transform_iter(&self) -> TransformIter<'_> {
		TransformIter {
			stack: vec![(self, glam::DAffine2::from_scale(DVec2::splat(0.1)), 0)],
		}
	}

	pub fn curve_iter(&self) -> impl Iterator<Item = (kurbo::BezPath, PathStyle, u32)> + '_ {
		fn glam_to_kurbo(transform: DAffine2) -> kurbo::Affine {
			kurbo::Affine::new(transform.to_cols_array())
		}
		self.transform_iter().filter_map(|(layer, transform, depth)| match &layer.data {
			LayerDataType::Folder(_) => None,
			LayerDataType::Shape(shape) => {
				let mut path = shape.path.clone();
				path.apply_affine(glam_to_kurbo(transform));
				Some((path, shape.style, depth))
			}
			LayerDataType::Text(_) => None, // TODO: Implement
		})
	}

	pub fn line_iter(&self) -> impl Iterator<Item = (Vec<(Vec2, Vec2)>, PathStyle, u32)> + '_ {
		//log::debug!("line_iter");
		self.curve_iter().map(|(path, style, depth)| {
			let mut vec = Vec::new();
			path.flatten(0.5, |segment| vec.push(segment));
			//log::trace!("flat {vec:?}");
			let mut paths = Vec::new();
			let mut state = None;
			for operation in vec {
				state = match (state, operation) {
					(None, PathEl::MoveTo(point)) => Some(point),
					(Some(first), PathEl::LineTo(second)) => {
						let point_to_vec = |point: kurbo::Point| glam::Vec2::new(point.x as f32, point.y as f32);
						paths.push((point_to_vec(first), point_to_vec(second)));
						//log::debug!("{operation:?}");
						Some(second)
					}
					(_, PathEl::ClosePath) => {
						paths.push((paths[0].0, paths.last().unwrap().1));
						None
					}
					(current, next) => unreachable!(format!("Bezier flattening returned non line segments {current:?} {next:?}")),
				}
			}
			//log::debug!("flat {paths:?}");
			(paths, style, depth)
		})
	}

	pub fn render(&mut self, transforms: &mut Vec<DAffine2>, view_mode: ViewMode) -> &str {
		if !self.visible {
			return "";
		}

		if self.cache_dirty {
			transforms.push(self.transform);
			self.thumbnail_cache.clear();
			self.data.render(&mut self.thumbnail_cache, transforms, view_mode);

			self.cache.clear();
			let _ = writeln!(self.cache, r#"<g transform="matrix("#);
			self.transform.to_cols_array().iter().enumerate().for_each(|(i, f)| {
				let _ = self.cache.write_str(&(f.to_string() + if i == 5 { "" } else { "," }));
			});
			let _ = write!(
				self.cache,
				r#")" style="mix-blend-mode: {}; opacity: {}">{}</g>"#,
				self.blend_mode.to_svg_style_name(),
				self.opacity,
				self.thumbnail_cache.as_str()
			);
			transforms.pop();
			self.cache_dirty = false;
		}

		self.cache.as_str()
	}

	pub fn intersects_quad(&self, quad: Quad, path: &mut Vec<LayerId>, intersections: &mut Vec<Vec<LayerId>>) {
		if !self.visible {
			return;
		}

		let transformed_quad = self.transform.inverse() * quad;
		self.data.intersects_quad(transformed_quad, path, intersections)
	}

	pub fn current_bounding_box_with_transform(&self, transform: DAffine2) -> Option<[DVec2; 2]> {
		self.data.bounding_box(transform)
	}

	pub fn current_bounding_box(&self) -> Option<[DVec2; 2]> {
		self.current_bounding_box_with_transform(self.transform)
	}

	pub fn as_folder_mut(&mut self) -> Result<&mut Folder, DocumentError> {
		match &mut self.data {
			LayerDataType::Folder(f) => Ok(f),
			_ => Err(DocumentError::NotAFolder),
		}
	}

	pub fn as_folder(&self) -> Result<&Folder, DocumentError> {
		match &self.data {
			LayerDataType::Folder(f) => Ok(f),
			_ => Err(DocumentError::NotAFolder),
		}
	}

	pub fn as_text_mut(&mut self) -> Result<&mut Text, DocumentError> {
		match &mut self.data {
			LayerDataType::Text(t) => Ok(t),
			_ => Err(DocumentError::NotText),
		}
	}

	pub fn as_text(&self) -> Result<&Text, DocumentError> {
		match &self.data {
			LayerDataType::Text(t) => Ok(t),
			_ => Err(DocumentError::NotText),
		}
	}
}

impl Clone for Layer {
	fn clone(&self) -> Self {
		Self {
			visible: self.visible,
			name: self.name.clone(),
			data: self.data.clone(),
			transform: self.transform,
			cache: String::new(),
			thumbnail_cache: String::new(),
			cache_dirty: true,
			blend_mode: self.blend_mode,
			opacity: self.opacity,
		}
	}
}

impl<'a> IntoIterator for &'a Layer {
	type Item = &'a Layer;
	type IntoIter = LayerIter<'a>;

	fn into_iter(self) -> Self::IntoIter {
		self.iter()
	}
}

#[derive(Debug, Default)]
pub struct LayerIter<'a> {
	pub stack: Vec<&'a Layer>,
}

impl<'a> Iterator for LayerIter<'a> {
	type Item = &'a Layer;

	fn next(&mut self) -> Option<Self::Item> {
		match self.stack.pop() {
			Some(layer) => {
				if let LayerDataType::Folder(folder) = &layer.data {
					let layers = folder.layers();
					self.stack.extend(layers);
				};
				Some(layer)
			}
			None => None,
		}
	}
}

#[derive(Debug, Default)]
pub struct TransformIter<'a> {
	pub stack: Vec<(&'a Layer, glam::DAffine2, u32)>,
}

impl<'a> Iterator for TransformIter<'a> {
	type Item = (&'a Layer, glam::DAffine2, u32);

	fn next(&mut self) -> Option<Self::Item> {
		match self.stack.pop() {
			Some((layer, transform, depth)) => {
				log::debug!("transform: {transform:?}");
				let new_transform = transform * layer.transform;
				if let LayerDataType::Folder(folder) = &layer.data {
					let layers = folder.layers();
					self.stack.extend(layers.iter().map(|x| (x, new_transform, depth + 1)));
					self.next()
				} else {
					Some((layer, new_transform, depth))
				}
			}
			None => None,
		}
	}
}
