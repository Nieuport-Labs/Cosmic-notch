// SPDX-License-Identifier: GPL-3.0-only

//! A wrapper that lays its content out at its natural size, regardless of how
//! much room the parent gives it, and reports that size whenever it changes.
//!
//! The island animates its outer size towards the natural size of whatever it
//! currently shows, so it needs to know that size before it is reached.

use cosmic::Element;
use cosmic::iced::advanced::layout::{self, Layout};
use cosmic::iced::advanced::widget::{Operation, Tree, tree};
use cosmic::iced::advanced::{Clipboard, Shell, Widget, mouse, overlay, renderer};
use cosmic::iced::{Event, Length, Rectangle, Size, Vector, window};

pub struct Measure<'a, Message> {
    content: Element<'a, Message>,
    on_measure: Box<dyn Fn(Size) -> Message + 'a>,
}

pub fn measure<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    on_measure: impl Fn(Size) -> Message + 'a,
) -> Measure<'a, Message> {
    Measure {
        content: content.into(),
        on_measure: Box::new(on_measure),
    }
}

#[derive(Default)]
struct State {
    natural: Size,
    reported: Option<Size>,
}

impl<Message> Widget<Message, cosmic::Theme, cosmic::Renderer> for Measure<'_, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &cosmic::Renderer,
        _limits: &layout::Limits,
    ) -> layout::Node {
        let unbounded = layout::Limits::new(Size::ZERO, Size::INFINITE);
        let child = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &unbounded);
        let natural = child.size();
        tree.state.downcast_mut::<State>().natural = natural;
        layout::Node::with_children(natural, vec![child])
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut cosmic::Renderer,
        theme: &cosmic::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().unwrap(),
            cursor,
            viewport,
        );
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &cosmic::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            operation,
        );
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &cosmic::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let Event::Window(window::Event::RedrawRequested(_)) = event {
            let state = tree.state.downcast_mut::<State>();
            if state.reported != Some(state.natural) {
                state.reported = Some(state.natural);
                shell.publish((self.on_measure)(state.natural));
            }
        }

        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &cosmic::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &cosmic::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, cosmic::Theme, cosmic::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }

    fn drag_destinations(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        renderer: &cosmic::Renderer,
        dnd_rectangles: &mut cosmic::iced::advanced::clipboard::DndDestinationRectangles,
    ) {
        self.content.as_widget().drag_destinations(
            &tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            dnd_rectangles,
        );
    }
}

impl<'a, Message: 'a> From<Measure<'a, Message>> for Element<'a, Message> {
    fn from(measure: Measure<'a, Message>) -> Self {
        Element::new(measure)
    }
}
