#version 450

layout(location = 0) in vec2 a_texcoord;

layout(location = 0) out vec4 color_attachment;

layout(set = 0, binding = 0) uniform sampler u_sampler;
layout(set = 0, binding = 1) uniform texture2D u_color;
layout(set = 0, binding = 2) uniform PostProcessUniforms {
  vec4 color_shift;
} postprocess_uniforms;

void main() {
  vec2 texcoord = gl_FragCoord.xy;

  vec4 in_color = texture(sampler2D(u_color, u_sampler), texcoord);

  float src_factor = postprocess_uniforms.color_shift.a;
  float dst_factor = 1.0 - src_factor;
  vec4 color_shifted = src_factor * postprocess_uniforms.color_shift
    + dst_factor * in_color;

  color_attachment = color_shifted;
}
