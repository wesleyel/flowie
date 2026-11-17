use flowie_math::math;
use gpui::{
    div, prelude::*, px, rgb, size, App, Application, Bounds, Context, Window, WindowBounds,
    WindowOptions,
};

struct Demo;

const FORMULAS: &[(&str, bool)] = &[
    (r"e^{i\pi}+1=0", false),
    (r"\frac{a}{b}+\sqrt{x^2+y^2}", false),
    (r"\int_0^\infty e^{-x^2}\,dx=\frac{\sqrt{\pi}}{2}", true),
    (r"\sum_{n=1}^{\infty}\frac{1}{n^2}=\frac{\pi^2}{6}", true),
    (r"\left(\begin{matrix}a&b\\c&d\end{matrix}\right)\mathbb{R}^n", true),
    (r"\frac{1}{", false),
];

impl Render for Demo {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0xffffff))
            .text_color(rgb(0x202020))
            .text_size(px(20.))
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .children(FORMULAS.iter().map(|(src, display)| {
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().w(px(80.)).child(if *display { "display" } else { "inline" }))
                    .child(math(*src, *display))
            }))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        flowie_math::load_fonts(cx);
        let bounds = Bounds::centered(None, size(px(720.), px(620.)), cx);
        cx.open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
            |_, cx| cx.new(|_| Demo),
        )
        .unwrap();
        cx.activate(true);
    });
}
