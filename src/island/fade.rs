// SPDX-License-Identifier: GPL-3.0-only

//! Fades and scales content in, for the island's content transition.
//!
//! The renderer has no layer opacity, so the fade works on colors: the text
//! and icon colors handed down while drawing are faded, and style closures in
//! the island multiply their colors by [`alpha`] — the opacity of the
//! innermost [`Fade`] currently drawing.

use std::cell::Cell;

use cosmic::Element;
use cosmic::iced::advanced::layout::{self, Layout};
use cosmic::iced::advanced::widget::{Operation, Tree};
use cosmic::iced::advanced::{Clipboard, Renderer as _, Shell, Widget, mouse, overlay, renderer};
use cosmic::iced::{Color, Event, Length, Rectangle, Size, Transformation, Vector};

thread_local! {
    static ALPHA: Cell<f32> = const { Cell::new(1.0) };
}

/// Opacity of the content being drawn; 1.0 outside of any [`Fade`].
pub fn alpha() -> f32 {
    ALPHA.with(Cell::get)
}

/// `color` with its opacity multiplied by the current fade.
pub fn faded(color: impl Into<Color>) -> Color {
    let color = color.into();
    Color {
        a: color.a * alpha(),
        ..color
    }
}

pub struct Fade<'a, Message> {
    content: Element<'a, Message>,
    opacity: f32,
    scale: f32,
}

/// Draws `content` at `opacity`, scaled by `scale` around its top center.
pub fn fade<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    opacity: f32,
    scale: f32,
) -> Fade<'a, Message> {
    Fade {
        content: content.into(),
        opacity: opacity.clamp(0.0, 1.0),
        scale,
    }
}

impl<Message> Widget<Message, cosmic::Theme, cosmic::Renderer> for Fade<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
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
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
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
        if self.opacity <= 0.01 {
            return;
        }
        let outer = alpha();
        let opacity = outer * self.opacity;
        let style = renderer::Style {
            text_color: Color {
                a: style.text_color.a * self.opacity,
                ..style.text_color
            },
            icon_color: Color {
                a: style.icon_color.a * self.opacity,
                ..style.icon_color
            },
            scale_factor: style.scale_factor,
        };
        ALPHA.with(|a| a.set(opacity));
        let bounds = layout.bounds();
        let draw = |renderer: &mut cosmic::Renderer| {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                &style,
                layout,
                cursor,
                viewport,
            );
        };
        if (self.scale - 1.0).abs() < 0.001 {
            draw(renderer);
        } else {
            let anchor = Vector::new(bounds.x + bounds.width / 2.0, bounds.y);
            let transformation = Transformation::translate(anchor.x, anchor.y)
                * Transformation::scale(self.scale)
                * Transformation::translate(-anchor.x, -anchor.y);
            renderer.with_layer(*viewport, |renderer| {
                renderer.with_transformation(transformation, draw);
            });
        }
        ALPHA.with(|a| a.set(outer));
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &cosmic::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
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
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
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
            layout,
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
            layout,
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
            layout,
            renderer,
            dnd_rectangles,
        );
    }
}

impl<'a, Message: 'a> From<Fade<'a, Message>> for Element<'a, Message> {
    fn from(fade: Fade<'a, Message>) -> Self {
        Element::new(fade)
    }
}
