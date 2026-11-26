use crate::cache::validate_binary_model;
use rbx_dom_weak::{
    Instance, WeakDom,
    types::{CFrame, Color3, Variant, Vector3},
};
use serde::Serialize;

const MAX_PARTS: usize = 192;
const MAX_LISTED_OBJECTS: usize = 80;
const MAX_OBJECTS: usize = 65_536;
const MAX_LABEL: usize = 120;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelPreview {
    parts: Vec<Part>,
    objects: Vec<Object>,
    object_count: usize,
    omitted_parts: usize,
}

#[derive(Serialize)]
struct Part {
    shape: &'static str,
    size: [f32; 3],
    position: [f32; 3],
    rotation: [[f32; 3]; 3],
    color: [f32; 3],
    opacity: f32,
}

#[derive(Serialize)]
struct Object {
    name: String,
    class: String,
    depth: usize,
    references: Vec<String>,
}

pub(crate) fn preview(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let dom = if bytes.starts_with(b"<roblox!") {
        validate_binary_model(bytes)?;
        rbx_binary::from_reader(bytes).map_err(|_| "This binary Roblox model is invalid.")?
    } else {
        // An asset preview only requires instance data and never resolves external entities.
        let bytes = bytes
            .strip_prefix(b"\xef\xbb\xbf")
            .unwrap_or(bytes)
            .trim_ascii_start();
        if bytes
            .windows(b"<!DOCTYPE".len())
            .any(|token| token == b"<!DOCTYPE")
        {
            return Err("Roblox asset previews do not support XML document types.".into());
        }
        rbx_xml::from_reader_default(bytes).map_err(|_| "The Roblox model XML is invalid.")?
    };
    let mut preview = ModelPreview {
        parts: Vec::new(),
        objects: Vec::new(),
        object_count: 0,
        omitted_parts: 0,
    };
    let mut stack: Vec<_> = dom
        .root()
        .children()
        .iter()
        .rev()
        .map(|id| (*id, 0))
        .collect();
    while let Some((id, depth)) = stack.pop() {
        let Some(item) = dom.get_by_ref(id) else {
            continue;
        };
        preview.object_count += 1;
        if preview.object_count > MAX_OBJECTS {
            return Err("This model exceeds the preview object limit.".into());
        }
        if preview.objects.len() < MAX_LISTED_OBJECTS {
            preview.objects.push(Object {
                name: label(&item.name),
                class: label(item.class.as_str()),
                depth,
                references: asset_references(item),
            });
        }
        if let Some(part) = part_of(item, &dom) {
            if preview.parts.len() < MAX_PARTS {
                preview.parts.push(part);
            } else {
                preview.omitted_parts += 1;
            }
        }
        stack.extend(item.children().iter().rev().map(|id| (*id, depth + 1)));
    }
    serde_json::to_vec(&preview).map_err(|_| "The model preview could not be prepared.".into())
}

fn vector(value: Vector3) -> [f32; 3] {
    [value.x, value.y, value.z]
}

fn part_of(item: &Instance, dom: &WeakDom) -> Option<Part> {
    let shape = match item.class.as_str() {
        "Part" => match item.properties.get(&"Shape".into()) {
            Some(Variant::Enum(kind)) => match kind.to_u32() {
                0 => "ball",
                1 => "block",
                2 => "cylinder",
                3 => "wedge",
                _ => "bounds",
            },
            _ => "block",
        },
        "WedgePart" => "wedge",
        "Seat" | "VehicleSeat" | "SpawnLocation" | "TrussPart" => "block",
        "MeshPart" | "UnionOperation" | "PartOperation" | "CornerWedgePart" => "bounds",
        _ => return None,
    };
    let shape = if item.children().iter().any(|id| {
        dom.get_by_ref(*id).is_some_and(|child| {
            matches!(
                child.class.as_str(),
                "SpecialMesh" | "BlockMesh" | "CylinderMesh"
            )
        })
    }) {
        "bounds"
    } else {
        shape
    };
    let Some(Variant::Vector3(size)) = item.properties.get(&"Size".into()) else {
        return None;
    };
    let size = vector(*size);
    let frame = match item.properties.get(&"CFrame".into()) {
        Some(Variant::CFrame(frame)) => *frame,
        _ => CFrame::identity(),
    };
    let position = vector(frame.position);
    let rotation = [
        vector(frame.orientation.x),
        vector(frame.orientation.y),
        vector(frame.orientation.z),
    ];
    if !size
        .iter()
        .all(|value| value.is_finite() && *value > 0.0 && *value <= 1.0e6)
        || !position
            .iter()
            .all(|value| value.is_finite() && value.abs() <= 1.0e7)
        || !rotation
            .iter()
            .flatten()
            .all(|value| value.is_finite() && value.abs() <= 1.001)
    {
        return None;
    }
    let color = match item.properties.get(&"Color".into()) {
        Some(Variant::Color3(value)) => *value,
        Some(Variant::Color3uint8(value)) => Color3::from(*value),
        _ => match item.properties.get(&"BrickColor".into()) {
            Some(Variant::BrickColor(value)) => Color3::from(value.to_color3uint8()),
            _ => Color3::new(0.64, 0.64, 0.64),
        },
    };
    let color = [color.r, color.g, color.b].map(|value| {
        if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.64
        }
    });
    let opacity = match item.properties.get(&"Transparency".into()) {
        Some(Variant::Float32(value)) if value.is_finite() => 1.0 - value.clamp(0.0, 1.0),
        _ => 1.0,
    };
    (opacity > 0.0).then_some(Part {
        shape,
        size,
        position,
        rotation,
        color,
        opacity,
    })
}

fn asset_references(item: &Instance) -> Vec<String> {
    const PROPERTIES: &[&str] = &[
        "MeshId",
        "MeshContent",
        "Texture",
        "TextureId",
        "TextureID",
        "TextureContent",
        "ColorMap",
        "ColorMapContent",
        "ShirtTemplate",
        "PantsTemplate",
        "Graphic",
        "SoundId",
        "AnimationId",
    ];
    let mut references = Vec::new();
    for property in PROPERTIES {
        let uri = match item.properties.get(&(*property).into()) {
            Some(Variant::ContentId(value)) => Some(value.as_str()),
            Some(Variant::Content(value)) => value.as_uri(),
            _ => None,
        };
        let Some(uri) = uri.filter(|uri| !uri.is_empty() && uri.len() <= 512) else {
            continue;
        };
        let value = if let Some(id) = uri.strip_prefix("rbxassetid://") {
            id.parse::<u64>()
                .ok()
                .filter(|id| *id > 0)
                .map(|id| format!("Asset {id}"))
        } else if uri.starts_with("rbxasset://") {
            Some(label(uri))
        } else {
            reqwest::Url::parse(uri).ok().and_then(|url| {
                let host = url.host_str()?;
                if host != "roblox.com" && !host.ends_with(".roblox.com") {
                    return None;
                }
                let id = url
                    .query_pairs()
                    .find(|(name, _)| name == "id")?
                    .1
                    .parse::<u64>()
                    .ok()?;
                (id > 0).then(|| format!("Asset {id}"))
            })
        };
        if let Some(value) = value
            && !references.contains(&value)
        {
            references.push(value);
        }
    }
    references
}

fn label(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control())
        .take(MAX_LABEL)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_geometry_and_references_in_xml_and_binary_models() {
        let xml = br#"<roblox version="4"><Item class="Model" referent="model"><Properties>
          <string name="Name">Preview fixture</string>
        </Properties><Item class="Part" referent="part"><Properties>
          <string name="Name">Platform</string>
          <Vector3 name="size"><X>4</X><Y>1</Y><Z>2</Z></Vector3>
          <CoordinateFrame name="CFrame"><X>3</X><Y>2</Y><Z>1</Z>
            <R00>0</R00><R01>0</R01><R02>1</R02>
            <R10>0</R10><R11>1</R11><R12>0</R12>
            <R20>-1</R20><R21>0</R21><R22>0</R22>
          </CoordinateFrame><token name="shape">2</token>
        </Properties></Item><Item class="Decal" referent="decal"><Properties>
          <Content name="Texture"><url>rbxassetid://1234</url></Content>
        </Properties></Item></Item></roblox>"#;
        let dom = rbx_xml::from_reader_default(xml.as_slice()).unwrap();
        let mut binary = Vec::new();
        rbx_binary::to_writer(&mut binary, &dom, dom.root().children()).unwrap();
        for bytes in [xml.as_slice(), &binary] {
            let output: serde_json::Value =
                serde_json::from_slice(&preview(bytes).unwrap()).unwrap();
            assert_eq!(output["objectCount"], 3);
            assert_eq!(output["parts"][0]["shape"], "cylinder");
            assert_eq!(
                output["parts"][0]["size"],
                serde_json::json!([4.0, 1.0, 2.0])
            );
            assert_eq!(
                output["parts"][0]["position"],
                serde_json::json!([3.0, 2.0, 1.0])
            );
            assert_eq!(
                output["parts"][0]["rotation"][0],
                serde_json::json!([0.0, 0.0, 1.0])
            );
            assert_eq!(output["objects"][0]["name"], "Preview fixture");
            assert_eq!(output["objects"][2]["references"][0], "Asset 1234");
        }
    }
}
