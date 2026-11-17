//! A row whose card backgrounds stretch to its tallest natural child.
use super::*;
use iced::advanced::{
    Clipboard, Layout, Shell, Widget, layout, overlay, renderer,
    widget::{Operation, Tree},
};
use iced::widget::Row;
use iced::{Event, Length, Rectangle, Renderer, Size, Vector, mouse};

pub(super) fn cards<'a>(
    left: Element<'a, Message>,
    right: Element<'a, Message>,
) -> Element<'a, Message> {
    Element::new(EqualHeightRow {
        row: row![left, right].spacing(16),
    })
}

struct EqualHeightRow<'a> {
    row: Row<'a, Message>,
}

impl Widget<Message, Theme, Renderer> for EqualHeightRow<'_> {
    fn size(&self) -> Size<Length> {
        self.row.size()
    }
    fn children(&self) -> Vec<Tree> {
        self.row.children()
    }
    fn diff(&self, tree: &mut Tree) {
        self.row.diff(tree);
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let measured = self.row.layout(tree, renderer, limits);
        let height = measured.size().height;
        // Keep each card's content at its natural size and position. Stretch
        // only its outer container, which paints the background and border.
        let children = measured
            .children()
            .iter()
            .map(|child| {
                layout::Node::with_children(
                    Size::new(child.size().width, height),
                    child.children().to_vec(),
                )
                .move_to(child.bounds().position())
            })
            .collect();
        layout::Node::with_children(measured.size(), children)
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.row
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.row.operate(tree, layout, renderer, operation);
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.row.update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.row
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        self.row
            .overlay(tree, layout, renderer, viewport, translation)
    }
}
