#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#endif

struct LightmapUniforms {
    light_anim_frames: [f32; 64],
    camera_pos: Vec4,
    time: f32,
}

struct BrushTextureExtension {
    lightmap_anim_a: u8,
    lightmap_anim_b: u8,
    lightmap_anim_c: u8,
    lightmap_anim_d: u8,
    lightmap_b: texture2d,
    lightmap_c: texture2d,
    lightmap_d: texture2d,
}

@group(0) @binding(2) var<uniform> lightmapsettings: LightmapUniforms;

@group(2) @binding(100)
var<uniform> brushtexext: BrushTextureExtension;

// Samples the lightmap, if any, and returns indirect illumination from it.
fn lightmap(map: texture2d, uv: vec2<f32>, exposure: f32, instance_index: u32) -> vec3<f32> {
    let packed_uv_rect = mesh[instance_index].lightmap_uv_rect;
    let uv_rect = vec4<f32>(vec4<u32>(
        packed_uv_rect.x & 0xffffu,
        packed_uv_rect.x >> 16u,
        packed_uv_rect.y & 0xffffu,
        packed_uv_rect.y >> 16u)) / 65535.0;

    let lightmap_uv = mix(uv_rect.xy, uv_rect.zw, uv);

    // Mipmapping lightmaps is usually a bad idea due to leaking across UV
    // islands, so there's no harm in using mip level 0 and it lets us avoid
    // control flow uniformity problems.
    //
    // TODO(pcwalton): Consider bicubic filtering.
    return textureSampleLevel(
        map,
        lightmaps_sampler,
        lightmap_uv,
        0.0
    ).rgb * exposure;
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    // generate a PbrInput struct from the StandardMaterial bindings
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // alpha discard
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    pbr_input.material.lightmap_color *= light_anim_frames[light_anim_a];
    pbr_input.material.lightmap_color += lightmap(
        brustexext.lightmap_b,
        in.uv_b,
        pbr_bindings::material.lightmap_exposure,
        in.instance_index
    ) * light_anim_frames[light_anim_a];
    pbr_input.material.lightmap_color += lightmap(
        brustexext.lightmap_c,
        in.uv_b,
        pbr_bindings::material.lightmap_exposure,
        in.instance_index
    ) * light_anim_frames[light_anim_b];
    pbr_input.material.lightmap_color += lightmap(
        brustexext.lightmap_c,
        in.uv_b,
        pbr_bindings::material.lightmap_exposure,
        in.instance_index
    ) * light_anim_frames[light_anim_c];

#ifdef PREPASS_PIPELINE
    // in deferred mode we can't modify anything after that, as lighting is run in a separate fullscreen shader.
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    // apply lighting
    out.color = apply_pbr_lighting(pbr_input);

    // we can optionally modify the lit color before post-processing is applied
    out.color = vec4<f32>(vec4<u32>(out.color * f32(my_extended_material.quantize_steps))) / f32(my_extended_material.quantize_steps);

    // apply in-shader post processing (fog, alpha-premultiply, and also tonemapping, debanding if the camera is non-hdr)
    // note this does not include fullscreen postprocessing effects like bloom.
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);

    // we can optionally modify the final result here
    out.color = out.color * 2.0;
#endif

    return out;
}

