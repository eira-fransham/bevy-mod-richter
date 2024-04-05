#version 450
#define LIGHTMAP_ANIM_END (255)

const uint TEXTURE_KIND_REGULAR = 0;
const uint TEXTURE_KIND_WARP = 1;
const uint TEXTURE_KIND_SKY = 2;

const float WARP_AMPLITUDE = 0.15;
const float WARP_FREQUENCY = 0.25;
const float WARP_SCALE = 1.0;

layout(location = 0) in vec3 f_normal;
layout(location = 1) in vec3 f_diffuse; // also used for fullbright, for sky textures this is the position instead
layout(location = 2) in vec2 f_lightmap;
flat layout(location = 3) in uvec4 f_lightmap_anim;

layout(push_constant) uniform PushConstants {
  layout(offset = 128) uint texture_kind;
} push_constants;

// set 0: per-frame
layout(set = 0, binding = 0) uniform FrameUniforms {
    vec4 light_anim_frames[16];
    vec4 camera_pos;
    float time;
    float sky_time;
} frame_uniforms;

// set 1: per-entity
layout(set = 1, binding = 1) uniform sampler u_diffuse_sampler; // also used for fullbright
layout(set = 1, binding = 2) uniform sampler u_lightmap_sampler;

// set 2: per-texture
layout(set = 2, binding = 0) uniform texture2D u_diffuse_texture;
layout(set = 2, binding = 1) uniform texture2D u_fullbright_texture;
layout(set = 2, binding = 2) uniform TextureUniforms {
    uint kind;
} texture_uniforms;

// set 3: per-face
layout(set = 3, binding = 0) uniform texture2D u_lightmap_texture[4];

layout(location = 0) out vec4 diffuse_attachment;
layout(location = 1) out vec4 normal_attachment;

vec4 calc_light() {
    vec4 light = vec4(0.0, 0.0, 0.0, 0.0);
    for (int i = 0; i < 4; i++) {
        if (f_lightmap_anim[i] == LIGHTMAP_ANIM_END)
            break;

        float map = texture(
            sampler2D(u_lightmap_texture[i], u_lightmap_sampler),
            f_lightmap
        ).r;

        // range [0, 4]
        ivec2 idx = ivec2(floor(f_lightmap_anim[i] / 4), mod(f_lightmap_anim[i], 4));
        float style = frame_uniforms.light_anim_frames[idx.x][idx.y];
        light[i] = map * style;
    }

    return light;
}

// Compute line-plane intersection
vec3 intersection(vec3 norm, vec3 plane_pos, vec3 plane_norm) {
    float plane_dot = dot(norm, plane_norm);
    vec3 w = f_diffuse - plane_pos;
    float factor = -dot(w, plane_norm) / plane_dot;

    return w + norm * factor + plane_pos;
}

// TODO: Convert this push constant to be separated shaders instead
void main() {
    switch (push_constants.texture_kind) {
        case TEXTURE_KIND_REGULAR:
            float fullbright = texture(
                sampler2D(u_fullbright_texture, u_diffuse_sampler),
                f_diffuse.xy
            ).r;


            float light;
            if (fullbright != 0.0) {
                light = 0.25;
            } else {
                light = dot(calc_light(), vec4(1.));
            }

            diffuse_attachment = vec4(texture(
                sampler2D(u_diffuse_texture, u_diffuse_sampler),
                f_diffuse.xy
            ).rgb, light);

            break;

        case TEXTURE_KIND_WARP:
            // note the texcoord transpose here
            vec2 wave1 = 3.14159265359
                * (WARP_SCALE * f_diffuse.ts
                    + WARP_FREQUENCY * frame_uniforms.time);

            vec2 warp_texcoord = f_diffuse.st + WARP_AMPLITUDE
                * vec2(sin(wave1.s), sin(wave1.t));

            diffuse_attachment = vec4(texture(
                sampler2D(u_diffuse_texture, u_diffuse_sampler),
                warp_texcoord
            ).rgb, 0.25);
            break;

        case TEXTURE_KIND_SKY:
            // TODO: Convert these into cvars?
            float sky_height = 1000.;
            float cloud_height = 700.;
            float sky_size = 6.;

            vec3 sky_plane_pos = vec3(0., 0., sky_height);
            vec3 cloud_plane_pos = vec3(0., 0., cloud_height);
            vec3 plane_norm = vec3(0., 0., -1);

            // We calculate the diffuse coords here instead of in the vertex shader to prevent incorrect
            // interpolation when the skybox is not parallel to the sky plane (e.g. for sky-textured walls)
            // TODO: Is there a more-efficient way to do this?
            vec3 dir = f_diffuse - frame_uniforms.camera_pos.xyz / frame_uniforms.camera_pos.w;

            vec2 size = vec2(textureSize(sampler2D(u_diffuse_texture, u_diffuse_sampler), 0));

            vec2 scroll = vec2(frame_uniforms.sky_time) / vec2(size.y);

            vec2 sky_coord = intersection(dir, sky_plane_pos, plane_norm).xy / sky_size / size ;
            vec2 cloud_coord = intersection(dir, cloud_plane_pos, plane_norm).xy / sky_size / size;

            sky_coord = mod(sky_coord + scroll, 1.) * vec2(0.5, 1.) + vec2(0.5, 0.);
            cloud_coord = mod(cloud_coord + scroll, 1.) * vec2(0.5, 1.);

            vec4 sky_color = texture(
                sampler2D(u_diffuse_texture, u_diffuse_sampler),
                sky_coord
            );
            vec4 cloud_color = texture(
                sampler2D(u_diffuse_texture, u_diffuse_sampler),
                cloud_coord
            );

            // 0.0 if black, 1.0 otherwise
            float cloud_factor;
            if (cloud_color.r + cloud_color.g + cloud_color.b == 0.0) {
                diffuse_attachment = vec4(sky_color.rgb, 0.25);
            } else {
                diffuse_attachment = vec4(cloud_color.rgb, 0.25);
            }
            break;

        // not possible
        default:
            break;
    }

    // rescale normal to [0, 1]
    normal_attachment = vec4(f_normal / 2.0 + 0.5, 1.0);
}
