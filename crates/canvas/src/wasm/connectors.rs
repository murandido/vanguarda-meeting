//! JS bindings for connectors (CA-03), kept apart from the core bindings.
//! Sides cross the boundary as `"n" | "e" | "s" | "w" | "auto"`; geometry as
//! flat arrays of [`STRIDE`] numbers per connector (see `connectors.js`).

use wasm_bindgen::prelude::*;

use super::{JsBoard, JsCamera};
use crate::connectors::{Endpoint, Route, STRIDE, Side};

/// Extra margin, in screen pixels, when culling connectors, so an arrowhead
/// sticking out of the curve's box is not clipped at the screen edge.
const CULL_MARGIN_PX: f64 = 16.0;

/// Where a new connector ends: on shape `shape` if given, else on `(x, y)`.
fn target(shape: Option<String>, side: &str, x: f64, y: f64) -> Endpoint {
    match shape {
        Some(id) => Endpoint::shape(&id, Side::parse(side)),
        None => Endpoint::Point { x, y },
    }
}

fn route(straight: bool) -> Route {
    if straight {
        Route::Straight
    } else {
        Route::Curved
    }
}

/// Anchor dot under the pointer: shape id and side.
#[wasm_bindgen]
pub struct Anchor {
    #[wasm_bindgen(getter_with_clone)]
    pub shape: String,
    #[wasm_bindgen(getter_with_clone)]
    pub side: String,
}

#[wasm_bindgen(js_class = Board)]
impl JsBoard {
    /// Numbers per connector in the geometry arrays below.
    #[wasm_bindgen(js_name = connectorStride)]
    #[must_use]
    pub fn connector_stride() -> usize {
        STRIDE
    }

    /// Creates connector `id` from a side of shape `from` to shape `to_shape`
    /// (side `to_side`) or, without `to_shape`, to the world point `(x, y)`.
    #[wasm_bindgen(js_name = addConnector)]
    #[allow(clippy::too_many_arguments)]
    pub fn add_connector(
        &self,
        id: &str,
        from: &str,
        from_side: &str,
        to_shape: Option<String>,
        to_side: &str,
        x: f64,
        y: f64,
    ) {
        self.0.add_connector(
            id,
            &Endpoint::shape(from, Side::parse(from_side)),
            &target(to_shape, to_side, x, y),
            Route::default(),
        );
    }

    /// Geometry of the connector being dragged out (empty if unresolvable).
    #[wasm_bindgen(js_name = previewConnector)]
    #[must_use]
    pub fn preview_connector(
        &self,
        from: &str,
        from_side: &str,
        to_shape: Option<String>,
        to_side: &str,
        x: f64,
        y: f64,
    ) -> Vec<f64> {
        self.0.preview_connector(
            &Endpoint::shape(from, Side::parse(from_side)),
            &target(to_shape, to_side, x, y),
            Route::default(),
        )
    }

    /// Switches connector `id` between curved and straight; returns whether
    /// it is straight now.
    #[wasm_bindgen(js_name = toggleConnectorRoute)]
    pub fn toggle_connector_route(&self, id: &str) -> bool {
        let straight = self
            .0
            .connectors()
            .iter()
            .any(|c| c.id == id && c.route == Route::Curved);
        self.0.set_connector_route(id, route(straight));
        straight
    }

    /// Connectors visible to `camera` on a `width` x `height` screen.
    #[wasm_bindgen(js_name = visibleConnectors)]
    #[must_use]
    pub fn visible_connectors(&self, camera: &JsCamera, width: f64, height: f64) -> Vec<f64> {
        let cam = &camera.0;
        self.0
            .visible_connectors(&cam.viewport(width, height), CULL_MARGIN_PX / cam.zoom())
    }

    /// Geometry of one connector (empty if missing or dangling).
    #[wasm_bindgen(js_name = connectorGeometry)]
    #[must_use]
    pub fn connector_geometry(&self, id: &str) -> Vec<f64> {
        self.0.connector_geometry(id)
    }

    /// Connector within `tolerance_px` screen pixels of world point `(x, y)`.
    #[wasm_bindgen(js_name = hitConnector)]
    #[must_use]
    pub fn hit_connector(
        &self,
        camera: &JsCamera,
        x: f64,
        y: f64,
        tolerance_px: f64,
    ) -> Option<String> {
        self.0.hit_connector(x, y, tolerance_px / camera.0.zoom())
    }

    /// World anchor points of shape `id` (N, E, S, W), empty if missing.
    #[wasm_bindgen(js_name = anchorPoints)]
    #[must_use]
    pub fn anchor_points(&self, id: &str) -> Vec<f64> {
        self.0.anchor_points(id)
    }

    /// Anchor dot within `tolerance_px` screen pixels of world point `(x, y)`.
    #[wasm_bindgen(js_name = anchorNear)]
    #[must_use]
    pub fn anchor_near(
        &self,
        camera: &JsCamera,
        x: f64,
        y: f64,
        tolerance_px: f64,
    ) -> Option<Anchor> {
        self.0
            .anchor_near(x, y, tolerance_px / camera.0.zoom())
            .map(|(shape, side)| Anchor {
                shape,
                side: Side::as_str(Some(side)).to_owned(),
            })
    }
}
