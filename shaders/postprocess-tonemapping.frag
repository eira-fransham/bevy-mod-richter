#version 450

layout(location = 0) in vec2 a_texcoord;

layout(location = 0) out vec4 color_attachment;

layout(set = 0, binding = 0) uniform sampler u_sampler;
layout(set = 0, binding = 1) uniform texture2D u_color;
layout(set = 0, binding = 2) uniform PostProcessUniforms {
  vec4 color_shift;
  float brightness;
  float inv_gamma;
} postprocess_uniforms;

const mat3 ToXYZMatrix = mat3(
    0.4124564, 0.3575761, 0.1804375,
    0.2126729, 0.7151522, 0.0721750,
    0.0193339, 0.1191920, 0.9503041
);

float luminance(vec3 color) {
    return dot(color, ToXYZMatrix[1]);
}

vec3 applyLuminance(vec3 color, float lum) {
    float originalLuminance = luminance(color);
    float scale = lum / originalLuminance;

    return color * scale;
}

float aces(float lum) {
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;

    lum = (lum * (a * lum + b)) / (lum * (c * lum + d) + e);

    return lum;
}

vec3 acesLum(vec3 color) {
    float lum = aces(luminance(color));

    return applyLuminance(color, lum);
}

vec4 acesLum(vec4 color) {
    return vec4(acesLum(vec3(color)), color.a);
}

vec3 aces(vec3 color) {
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;

    return (color * (a * color + b)) / (color * (c * color + d) + e);
}

const float a_CrosstalkSaturation = 2.0;
const float a_Saturation = 1.1;
const float a_InvCrosstalkAmt = 1.7;

vec3 crosstalk(vec3 tonemapped) {
    float tonemappedMax = max(tonemapped.r, max(tonemapped.g, tonemapped.b));
    vec3 ratio = tonemapped / tonemappedMax;
    tonemappedMax = min(tonemappedMax, 1.0);

    ratio = pow(ratio, vec3(a_Saturation / a_CrosstalkSaturation));
    ratio = mix(ratio, vec3(1.0), pow(tonemappedMax, a_InvCrosstalkAmt));
    ratio = pow(ratio, vec3(a_CrosstalkSaturation));

    return ratio * tonemappedMax;
}

vec3 crosstalkLum(vec3 tonemapped) {
    float tonemappedMax = luminance(tonemapped);
    vec3 ratio = tonemapped / tonemappedMax;
    tonemappedMax = min(tonemappedMax, 1.0);

    ratio = pow(ratio, vec3(a_Saturation / a_CrosstalkSaturation));
    ratio = mix(ratio, vec3(1.0), pow(tonemappedMax, a_InvCrosstalkAmt));
    ratio = pow(ratio, vec3(a_CrosstalkSaturation));

    return ratio * tonemappedMax;
}

const mat3 FromXYZMatrix = mat3(
    3.2404542, -1.5371385, -0.4985314,
    -0.9692660, 1.8760108, 0.0415560,
    0.0556434, -0.2040259, 1.0572252
);

void main() {
  vec4 in_color = texture(sampler2D(u_color, u_sampler), a_texcoord);

  bool tonemapping = true;
  bool crosstalkEnabled = true;
  bool xyySpaceCrosstalk = true;

  float src_factor = postprocess_uniforms.color_shift.a;
  float dst_factor = 1.0 - src_factor;
  vec3 color_shifted =
    FromXYZMatrix * (
      (ToXYZMatrix * src_factor * postprocess_uniforms.color_shift.rgb)
        + (ToXYZMatrix * dst_factor * in_color.rgb)
    );

  vec3 final;
  if (tonemapping) {
    // We want to allow RGB>1 but we also want to fix degenerate/non-finite values
    vec3 tonemapped = clamp(
        acesLum(color_shifted * postprocess_uniforms.brightness),
        vec3(0.0),
        vec3(100.0)
    );

    if (crosstalkEnabled) {
        final = xyySpaceCrosstalk ? crosstalkLum(tonemapped) : crosstalk(tonemapped);
    } else {
        final = tonemapped;
    }
  } else {
    final = color_shifted;
  }

  color_attachment = vec4(clamp(pow(final.rgb, vec3(postprocess_uniforms.inv_gamma)), vec3(0.), vec3(1.)), in_color.a);
}
