use super::shared::resize::Resize;
use crate::consts::DRAG_THRESHOLD;
use crate::frontend::utility_types::MouseCursorIcon;
use crate::input::keyboard::{Key, MouseMotion};
use crate::layout::widgets::PropertyHolder;
use crate::message_prelude::*;
use crate::misc::{HintData, HintGroup, HintInfo, KeysGroup};
use crate::viewport_tools::tool::{Fsm, SignalToMessageMap, ToolActionHandlerData, ToolMetadata, ToolTransition, ToolType};

use graphene::layers::style;
use graphene::Operation;

use glam::DAffine2;
use serde::{Deserialize, Serialize};

#[derive(Default)]
pub struct RectangleTool {
	fsm_state: RectangleToolFsmState,
	tool_data: RectangleToolData,
}

#[remain::sorted]
#[impl_message(Message, ToolMessage, Rectangle)]
#[derive(PartialEq, Eq, Clone, Debug, Hash, Serialize, Deserialize)]
pub enum RectangleToolMessage {
	// Standard messages
	#[remain::unsorted]
	Abort,

	// Tool-specific messages
	DragStart,
	DragStop,
	Resize {
		center: Key,
		lock_ratio: Key,
	},
}

impl PropertyHolder for RectangleTool {}

impl<'a> MessageHandler<ToolMessage, ToolActionHandlerData<'a>> for RectangleTool {
	fn process_action(&mut self, action: ToolMessage, tool_data: ToolActionHandlerData<'a>, responses: &mut VecDeque<Message>) {
		if action == ToolMessage::UpdateHints {
			self.fsm_state.update_hints(responses);
			return;
		}

		if action == ToolMessage::UpdateCursor {
			self.fsm_state.update_cursor(responses);
			return;
		}

		let new_state = self.fsm_state.transition(action, &mut self.tool_data, tool_data, &(), responses);

		if self.fsm_state != new_state {
			self.fsm_state = new_state;
			self.fsm_state.update_hints(responses);
			self.fsm_state.update_cursor(responses);
		}
	}

	fn actions(&self) -> ActionList {
		use RectangleToolFsmState::*;

		match self.fsm_state {
			Ready => actions!(RectangleToolMessageDiscriminant;
				DragStart,
			),
			Drawing => actions!(RectangleToolMessageDiscriminant;
				DragStop,
				Abort,
				Resize,
			),
		}
	}
}

impl ToolMetadata for RectangleTool {
	fn icon_name(&self) -> String {
		"VectorRectangleTool".into()
	}
	fn tooltip(&self) -> String {
		"Rectangle Tool (M)".into()
	}
	fn tool_type(&self) -> crate::viewport_tools::tool::ToolType {
		ToolType::Rectangle
	}
}

impl ToolTransition for RectangleTool {
	fn signal_to_message_map(&self) -> SignalToMessageMap {
		SignalToMessageMap {
			document_dirty: None,
			tool_abort: Some(RectangleToolMessage::Abort.into()),
			selection_changed: None,
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RectangleToolFsmState {
	Ready,
	Drawing,
}

impl Default for RectangleToolFsmState {
	fn default() -> Self {
		RectangleToolFsmState::Ready
	}
}
#[derive(Clone, Debug, Default)]
struct RectangleToolData {
	data: Resize,
}

impl Fsm for RectangleToolFsmState {
	type ToolData = RectangleToolData;
	type ToolOptions = ();

	fn transition(
		self,
		event: ToolMessage,
		tool_data: &mut Self::ToolData,
		(document, global_tool_data, input, font_cache): ToolActionHandlerData,
		_tool_options: &Self::ToolOptions,
		responses: &mut VecDeque<Message>,
	) -> Self {
		use RectangleToolFsmState::*;
		use RectangleToolMessage::*;

		let mut shape_data = &mut tool_data.data;

		if let ToolMessage::Rectangle(event) = event {
			match (self, event) {
				(Ready, DragStart) => {
					shape_data.start(responses, document, input.mouse.position, font_cache);
					responses.push_back(DocumentMessage::StartTransaction.into());
					shape_data.path = Some(document.get_path_for_new_layer());
					responses.push_back(DocumentMessage::DeselectAllLayers.into());

					responses.push_back(
						Operation::AddRect {
							path: shape_data.path.clone().unwrap(),
							insert_index: -1,
							transform: DAffine2::ZERO.to_cols_array(),
							style: style::PathStyle::new(None, style::Fill::solid(global_tool_data.primary_color)),
						}
						.into(),
					);

					Drawing
				}
				(state, Resize { center, lock_ratio }) => {
					if let Some(message) = shape_data.calculate_transform(responses, document, center, lock_ratio, input) {
						responses.push_back(message);
					}

					state
				}
				(Drawing, DragStop) => {
					match shape_data.drag_start.distance(input.mouse.position) <= DRAG_THRESHOLD {
						true => responses.push_back(DocumentMessage::AbortTransaction.into()),
						false => responses.push_back(DocumentMessage::CommitTransaction.into()),
					}

					shape_data.cleanup(responses);

					Ready
				}
				(Drawing, Abort) => {
					responses.push_back(DocumentMessage::AbortTransaction.into());

					shape_data.cleanup(responses);

					Ready
				}
				_ => self,
			}
		} else {
			self
		}
	}

	fn update_hints(&self, responses: &mut VecDeque<Message>) {
		let hint_data = match self {
			RectangleToolFsmState::Ready => HintData(vec![HintGroup(vec![
				HintInfo {
					key_groups: vec![],
					mouse: Some(MouseMotion::LmbDrag),
					label: String::from("Draw Rectangle"),
					plus: false,
				},
				HintInfo {
					key_groups: vec![KeysGroup(vec![Key::KeyShift])],
					mouse: None,
					label: String::from("Constrain Square"),
					plus: true,
				},
				HintInfo {
					key_groups: vec![KeysGroup(vec![Key::KeyAlt])],
					mouse: None,
					label: String::from("From Center"),
					plus: true,
				},
			])]),
			RectangleToolFsmState::Drawing => HintData(vec![HintGroup(vec![
				HintInfo {
					key_groups: vec![KeysGroup(vec![Key::KeyShift])],
					mouse: None,
					label: String::from("Constrain Square"),
					plus: false,
				},
				HintInfo {
					key_groups: vec![KeysGroup(vec![Key::KeyAlt])],
					mouse: None,
					label: String::from("From Center"),
					plus: false,
				},
			])]),
		};

		responses.push_back(FrontendMessage::UpdateInputHints { hint_data }.into());
	}

	fn update_cursor(&self, responses: &mut VecDeque<Message>) {
		responses.push_back(FrontendMessage::UpdateMouseCursor { cursor: MouseCursorIcon::Crosshair }.into());
	}
}