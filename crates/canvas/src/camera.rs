//! Câmera do quadro infinito (CA-01): deslocamento + zoom.
//!
//! A câmera é estado de visualização **local** de cada participante — não vai
//! para o documento Loro e nunca é sincronizada. Convenção: `(x, y)` é o ponto
//! do mundo que aparece no canto superior esquerdo da tela, e `zoom` é quantos
//! pixels de tela correspondem a uma unidade do mundo. Assim:
//!
//! ```text
//! tela  = (mundo - (x, y)) * zoom
//! mundo = tela / zoom + (x, y)
//! ```

/// Menor zoom permitido (10%).
pub const MIN_ZOOM: f64 = 0.1;
/// Maior zoom permitido (400%).
pub const MAX_ZOOM: f64 = 4.0;

/// Sensibilidade do zoom pela roda/pinça: fator `e^(-delta * k)`.
const WHEEL_ZOOM_SPEED: f64 = 0.01;
/// Limite do `deltaY` por evento, para um "clique" de roda de mouse (~100px)
/// não pular de zoom de uma vez; a pinça do trackpad manda deltas pequenos.
const WHEEL_ZOOM_MAX_DELTA: f64 = 20.0;
/// Pixels por linha quando o navegador reporta `deltaMode` em linhas.
const WHEEL_LINE_PX: f64 = 16.0;
/// Pixels por página quando o navegador reporta `deltaMode` em páginas.
const WHEEL_PAGE_PX: f64 = 800.0;

/// Caixa alinhada aos eixos, em coordenadas do mundo (intervalos fechados).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Bounds {
    /// Caixa a partir de origem e tamanho, como os retângulos do quadro.
    #[must_use]
    pub fn from_rect(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self {
            min_x: x,
            min_y: y,
            max_x: x + w,
            max_y: y + h,
        }
    }

    #[must_use]
    pub fn width(&self) -> f64 {
        self.max_x - self.min_x
    }

    #[must_use]
    pub fn height(&self) -> f64 {
        self.max_y - self.min_y
    }

    /// Verdadeiro se as caixas se tocam ou se sobrepõem.
    #[must_use]
    pub fn intersects(&self, other: &Self) -> bool {
        self.min_x <= other.max_x
            && other.min_x <= self.max_x
            && self.min_y <= other.max_y
            && other.min_y <= self.max_y
    }

    #[must_use]
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.min_x && x <= self.max_x && y >= self.min_y && y <= self.max_y
    }

    /// Menor caixa que contém as duas.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }
}

/// Câmera de um participante. Começa na origem com zoom 100%, de modo que
/// coordenadas do mundo e da tela coincidem até o primeiro pan/zoom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    x: f64,
    y: f64,
    zoom: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

/// Limita o zoom ao intervalo permitido; valores não finitos viram `None`.
fn clamp_zoom(zoom: f64) -> Option<f64> {
    (zoom.is_finite() && zoom > 0.0).then(|| zoom.clamp(MIN_ZOOM, MAX_ZOOM))
}

impl Camera {
    /// Câmera com o canto superior esquerdo em `(x, y)`; o zoom é limitado a
    /// [`MIN_ZOOM`]..=[`MAX_ZOOM`] (inválido vira 100%).
    #[must_use]
    pub fn new(x: f64, y: f64, zoom: f64) -> Self {
        Self {
            x,
            y,
            zoom: clamp_zoom(zoom).unwrap_or(1.0),
        }
    }

    #[must_use]
    pub fn x(&self) -> f64 {
        self.x
    }

    #[must_use]
    pub fn y(&self) -> f64 {
        self.y
    }

    #[must_use]
    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    /// Ponto da tela (pixels CSS) → ponto do mundo.
    #[must_use]
    pub fn screen_to_world(&self, sx: f64, sy: f64) -> (f64, f64) {
        (self.x + sx / self.zoom, self.y + sy / self.zoom)
    }

    /// Ponto do mundo → ponto da tela (pixels CSS).
    #[must_use]
    pub fn world_to_screen(&self, wx: f64, wy: f64) -> (f64, f64) {
        ((wx - self.x) * self.zoom, (wy - self.y) * self.zoom)
    }

    /// Região do mundo visível numa tela de `width` x `height` pixels.
    #[must_use]
    pub fn viewport(&self, width: f64, height: f64) -> Bounds {
        let (max_x, max_y) = self.screen_to_world(width, height);
        Bounds {
            min_x: self.x,
            min_y: self.y,
            max_x,
            max_y,
        }
    }

    /// Arrasta o conteúdo `(dx, dy)` pixels de tela (positivo = conteúdo vai
    /// para a direita/baixo, como num arraste com a mão).
    pub fn pan(&mut self, dx: f64, dy: f64) {
        if dx.is_finite() && dy.is_finite() {
            self.x -= dx / self.zoom;
            self.y -= dy / self.zoom;
        }
    }

    /// Define o zoom mantendo fixo o ponto do mundo sob `(sx, sy)` na tela.
    /// O zoom é limitado ao intervalo permitido; valores inválidos são ignorados.
    pub fn set_zoom_at(&mut self, sx: f64, sy: f64, zoom: f64) {
        let Some(zoom) = clamp_zoom(zoom) else {
            return;
        };
        let (wx, wy) = self.screen_to_world(sx, sy);
        self.zoom = zoom;
        self.x = wx - sx / zoom;
        self.y = wy - sy / zoom;
    }

    /// Multiplica o zoom por `factor` em torno de `(sx, sy)` na tela.
    pub fn zoom_at(&mut self, sx: f64, sy: f64, factor: f64) {
        self.set_zoom_at(sx, sy, self.zoom * factor);
    }

    /// Zoom por um evento de roda com Ctrl/Cmd ou pinça do trackpad, em torno
    /// do ponteiro. `delta_mode` segue o `WheelEvent.deltaMode` do DOM.
    pub fn wheel_zoom(&mut self, sx: f64, sy: f64, delta_y: f64, delta_mode: u32) {
        self.zoom_at(sx, sy, wheel_zoom_factor(delta_y, delta_mode));
    }

    /// Volta para 100% mantendo fixo o centro da tela.
    pub fn reset_zoom(&mut self, width: f64, height: f64) {
        self.set_zoom_at(width / 2.0, height / 2.0, 1.0);
    }

    /// Enquadra `content` numa tela de `width` x `height`, com `margin` pixels
    /// de folga em cada lado, centralizando o conteúdo.
    pub fn fit(&mut self, content: &Bounds, width: f64, height: f64, margin: f64) {
        let finite = [content.min_x, content.min_y, content.max_x, content.max_y]
            .iter()
            .all(|v| v.is_finite());
        if !(finite && width > 0.0 && height > 0.0) {
            return;
        }
        let usable = |side: f64| {
            let inner = side - 2.0 * margin;
            if inner > 0.0 { inner } else { side }
        };
        let (avail_w, avail_h) = (usable(width), usable(height));
        // Conteúdo sem largura/altura (um ponto ou uma linha) dá divisão por
        // zero = infinito, que o clamp transforma no zoom máximo.
        let zoom_w = if content.width() > 0.0 {
            avail_w / content.width()
        } else {
            f64::INFINITY
        };
        let zoom_h = if content.height() > 0.0 {
            avail_h / content.height()
        } else {
            f64::INFINITY
        };
        let zoom = zoom_w.min(zoom_h).clamp(MIN_ZOOM, MAX_ZOOM);
        let cx = f64::midpoint(content.min_x, content.max_x);
        let cy = f64::midpoint(content.min_y, content.max_y);
        self.zoom = zoom;
        self.x = cx - width / 2.0 / zoom;
        self.y = cy - height / 2.0 / zoom;
    }
}

/// Fator multiplicativo de zoom para um `deltaY` de roda: negativo aproxima,
/// positivo afasta. Simétrico: `f(d) * f(-d) == 1`.
#[must_use]
pub fn wheel_zoom_factor(delta_y: f64, delta_mode: u32) -> f64 {
    let px = match delta_mode {
        1 => delta_y * WHEEL_LINE_PX,
        2 => delta_y * WHEEL_PAGE_PX,
        _ => delta_y,
    };
    if !px.is_finite() {
        return 1.0;
    }
    let px = px.clamp(-WHEEL_ZOOM_MAX_DELTA, WHEEL_ZOOM_MAX_DELTA);
    (-px * WHEEL_ZOOM_SPEED).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9 * (1.0 + a.abs().max(b.abs()))
    }

    fn assert_point(actual: (f64, f64), expected: (f64, f64)) {
        assert!(
            close(actual.0, expected.0) && close(actual.1, expected.1),
            "esperado {expected:?}, obtido {actual:?}"
        );
    }

    #[test]
    fn default_is_identity() {
        let cam = Camera::default();
        assert_point(cam.screen_to_world(12.0, -7.5), (12.0, -7.5));
        assert_point(cam.world_to_screen(12.0, -7.5), (12.0, -7.5));
        assert!(close(cam.zoom(), 1.0));
    }

    #[test]
    fn new_clamps_and_rejects_invalid_zoom() {
        assert!(close(Camera::new(0.0, 0.0, 100.0).zoom(), MAX_ZOOM));
        assert!(close(Camera::new(0.0, 0.0, 0.001).zoom(), MIN_ZOOM));
        assert!(close(Camera::new(0.0, 0.0, f64::NAN).zoom(), 1.0));
        assert!(close(Camera::new(0.0, 0.0, -2.0).zoom(), 1.0));
        let cam = Camera::new(3.0, 4.0, 2.0);
        assert_eq!((cam.x(), cam.y(), cam.zoom()), (3.0, 4.0, 2.0));
    }

    #[test]
    fn conversions_with_offset_and_zoom() {
        let cam = Camera::new(100.0, 50.0, 2.0);
        assert_point(cam.screen_to_world(0.0, 0.0), (100.0, 50.0));
        assert_point(cam.screen_to_world(200.0, 100.0), (200.0, 100.0));
        assert_point(cam.world_to_screen(200.0, 100.0), (200.0, 100.0));
        assert_point(cam.world_to_screen(100.0, 50.0), (0.0, 0.0));
    }

    #[test]
    fn viewport_covers_the_screen() {
        let cam = Camera::new(-10.0, 20.0, 0.5);
        assert_eq!(
            cam.viewport(800.0, 600.0),
            Bounds {
                min_x: -10.0,
                min_y: 20.0,
                max_x: 1590.0,
                max_y: 1220.0
            }
        );
    }

    #[test]
    fn pan_moves_content_with_the_hand() {
        let mut cam = Camera::new(0.0, 0.0, 2.0);
        let before = cam.world_to_screen(10.0, 10.0);
        cam.pan(30.0, -40.0);
        let after = cam.world_to_screen(10.0, 10.0);
        assert_point(after, (before.0 + 30.0, before.1 - 40.0));
        // Deslocamentos inválidos não estragam a câmera.
        cam.pan(f64::NAN, 1.0);
        cam.pan(1.0, f64::INFINITY);
        assert_point(cam.world_to_screen(10.0, 10.0), after);
    }

    #[test]
    fn zoom_at_keeps_point_under_pointer() {
        let mut cam = Camera::new(37.0, -12.0, 1.3);
        let pointer = (412.0, 233.0);
        let world = cam.screen_to_world(pointer.0, pointer.1);
        for factor in [1.5, 0.25, 3.0, 0.9] {
            cam.zoom_at(pointer.0, pointer.1, factor);
            assert_point(cam.world_to_screen(world.0, world.1), pointer);
        }
    }

    #[test]
    fn zoom_is_clamped_and_still_anchored() {
        let mut cam = Camera::default();
        let world = cam.screen_to_world(300.0, 200.0);
        cam.zoom_at(300.0, 200.0, 1000.0);
        assert!(close(cam.zoom(), MAX_ZOOM));
        assert_point(cam.world_to_screen(world.0, world.1), (300.0, 200.0));
        cam.zoom_at(300.0, 200.0, 1e-6);
        assert!(close(cam.zoom(), MIN_ZOOM));
        assert_point(cam.world_to_screen(world.0, world.1), (300.0, 200.0));
    }

    #[test]
    fn invalid_zoom_is_ignored() {
        let mut cam = Camera::new(5.0, 6.0, 2.0);
        let before = cam;
        cam.set_zoom_at(10.0, 10.0, f64::NAN);
        cam.set_zoom_at(10.0, 10.0, 0.0);
        cam.zoom_at(10.0, 10.0, f64::INFINITY);
        assert_eq!(cam, before);
    }

    #[test]
    fn reset_zoom_keeps_center() {
        let mut cam = Camera::new(40.0, 80.0, 0.37);
        let center = cam.screen_to_world(400.0, 300.0);
        cam.reset_zoom(800.0, 600.0);
        assert!(close(cam.zoom(), 1.0));
        assert_point(cam.world_to_screen(center.0, center.1), (400.0, 300.0));
    }

    #[test]
    fn fit_centers_and_frames_content() {
        let mut cam = Camera::default();
        let content = Bounds::from_rect(1000.0, 1000.0, 400.0, 100.0);
        cam.fit(&content, 1000.0, 600.0, 100.0);
        // Largura limita: (1000 - 200) / 400 = 2.
        assert!(close(cam.zoom(), 2.0));
        assert_point(cam.world_to_screen(1200.0, 1050.0), (500.0, 300.0));
        assert_point(cam.world_to_screen(1000.0, 1000.0), (100.0, 200.0));
    }

    #[test]
    fn fit_uses_the_limiting_side_and_clamps() {
        let mut cam = Camera::default();
        // Altura limita: (600 - 0) / 3000 = 0.2.
        cam.fit(
            &Bounds::from_rect(0.0, 0.0, 100.0, 3000.0),
            1000.0,
            600.0,
            0.0,
        );
        assert!(close(cam.zoom(), 0.2));
        // Conteúdo enorme: preso em 10%.
        cam.fit(&Bounds::from_rect(0.0, 0.0, 1e9, 1e9), 1000.0, 600.0, 20.0);
        assert!(close(cam.zoom(), MIN_ZOOM));
        // Post-it pequeno: preso em 400%.
        cam.fit(
            &Bounds::from_rect(0.0, 0.0, 10.0, 10.0),
            1000.0,
            600.0,
            20.0,
        );
        assert!(close(cam.zoom(), MAX_ZOOM));
    }

    #[test]
    fn fit_degenerate_content_and_screens() {
        let mut cam = Camera::default();
        // Um ponto: zoom máximo, centrado nele.
        cam.fit(&Bounds::from_rect(50.0, 50.0, 0.0, 0.0), 800.0, 600.0, 40.0);
        assert!(close(cam.zoom(), MAX_ZOOM));
        assert_point(cam.world_to_screen(50.0, 50.0), (400.0, 300.0));
        // Margem maior que a tela: ignora a margem.
        cam.fit(&Bounds::from_rect(0.0, 0.0, 100.0, 100.0), 50.0, 50.0, 40.0);
        assert!(close(cam.zoom(), 0.5));
        // Tela sem tamanho: nada muda.
        let before = cam;
        cam.fit(&Bounds::from_rect(0.0, 0.0, 100.0, 100.0), 0.0, 0.0, 0.0);
        assert_eq!(cam, before);
        // Caixa inválida (NaN): nada muda.
        cam.fit(
            &Bounds::from_rect(f64::NAN, 0.0, f64::NAN, 1.0),
            800.0,
            600.0,
            0.0,
        );
        assert_eq!(cam, before);
    }

    #[test]
    fn wheel_factor_direction_symmetry_and_limits() {
        assert!(wheel_zoom_factor(-5.0, 0) > 1.0);
        assert!(wheel_zoom_factor(5.0, 0) < 1.0);
        assert!(close(wheel_zoom_factor(0.0, 0), 1.0));
        assert!(close(
            wheel_zoom_factor(7.0, 0) * wheel_zoom_factor(-7.0, 0),
            1.0
        ));
        // Roda de mouse (100px) é limitada a um passo moderado.
        assert!(close(
            wheel_zoom_factor(-100.0, 0),
            wheel_zoom_factor(-20.0, 0)
        ));
        // Linhas e páginas viram pixels antes do limite.
        assert!(close(wheel_zoom_factor(1.0, 1), wheel_zoom_factor(16.0, 0)));
        assert!(close(
            wheel_zoom_factor(-1.0, 2),
            wheel_zoom_factor(-20.0, 0)
        ));
        assert!(close(wheel_zoom_factor(f64::NAN, 0), 1.0));
    }

    #[test]
    fn wheel_zoom_anchors_at_pointer() {
        let mut cam = Camera::default();
        let world = cam.screen_to_world(640.0, 400.0);
        cam.wheel_zoom(640.0, 400.0, -10.0, 0);
        assert!(cam.zoom() > 1.0);
        assert_point(cam.world_to_screen(world.0, world.1), (640.0, 400.0));
    }

    #[test]
    fn bounds_helpers() {
        let a = Bounds::from_rect(0.0, 0.0, 10.0, 10.0);
        let b = Bounds::from_rect(10.0, 5.0, 5.0, 5.0);
        let c = Bounds::from_rect(11.0, 0.0, 1.0, 1.0);
        assert!(
            a.intersects(&b) && b.intersects(&a),
            "bordas encostadas contam"
        );
        assert!(!a.intersects(&c) && !c.intersects(&a));
        assert!(!a.intersects(&Bounds::from_rect(0.0, 11.0, 1.0, 1.0)));
        assert!(a.contains(10.0, 0.0) && !a.contains(10.1, 0.0));
        assert_eq!(a.union(&c), Bounds::from_rect(0.0, 0.0, 12.0, 10.0));
        assert!(close(b.width(), 5.0) && close(b.height(), 5.0));
    }
}
