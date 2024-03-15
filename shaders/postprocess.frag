#version 450

layout(location = 0) in vec2 a_texcoord;

layout(location = 0) out vec4 color_attachment;

layout(set = 0, binding = 0) uniform sampler u_sampler;
layout(set = 0, binding = 1) uniform texture2D u_color;
layout(set = 0, binding = 2) uniform PostProcessUniforms {
  vec4 color_shift[4];
  int blend_mode;
  uint color_space;
} postprocess_uniforms;

/*
GLSL Color Space Utility Functions
(c) 2015 tobspr

-------------------------------------------------------------------------------

The MIT License (MIT)

Copyright (c) 2015

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

-------------------------------------------------------------------------------

Most formulars / matrices are from:
https://en.wikipedia.org/wiki/SRGB

Some are from:
http://www.chilliant.com/rgb2hsv.html
https://www.fourcc.org/fccyvrgb.php
*/


// Define saturation macro, if not already user-defined
#ifndef saturate
#define saturate(v) clamp(v, 0, 1)
#endif

// Constants
const float HCV_EPSILON = 1e-10;
const float HSL_EPSILON = 1e-10;
const float HCY_EPSILON = 1e-10;

const float SRGB_GAMMA = 1.0 / 2.2;
const float SRGB_INVERSE_GAMMA = 2.2;
const float SRGB_ALPHA = 0.055;


// Used to convert from linear RGB to XYZ space
const mat3 RGB_2_XYZ = (mat3(
    0.4124564, 0.2126729, 0.0193339,
    0.3575761, 0.7151522, 0.1191920,
    0.1804375, 0.0721750, 0.9503041
));

// Used to convert from XYZ to linear RGB space
const mat3 XYZ_2_RGB = (mat3(
     3.2404542,-0.9692660, 0.0556434,
    -1.5371385, 1.8760108,-0.2040259,
    -0.4985314, 0.0415560, 1.0572252
));

const vec3 LUMA_COEFFS = vec3(0.2126, 0.7152, 0.0722);

// Returns the luminance of a !! linear !! rgb color
float get_luminance(vec3 rgb) {
    return dot(LUMA_COEFFS, rgb);
}

// Converts a linear rgb color to a srgb color (approximated, but fast)
vec3 rgb_to_srgb_approx(vec3 rgb) {
    return pow(rgb, vec3(SRGB_GAMMA));
}

// Converts a srgb color to a rgb color (approximated, but fast)
vec3 srgb_to_rgb_approx(vec3 srgb) {
    return pow(srgb, vec3(SRGB_INVERSE_GAMMA));
}

// Converts a single linear channel to srgb
float linear_to_srgb(float channel) {
    if(channel <= 0.0031308)
        return 12.92 * channel;
    else
        return (1.0 + SRGB_ALPHA) * pow(channel, 1.0/2.4) - SRGB_ALPHA;
}

// Converts a single srgb channel to rgb
float srgb_to_linear(float channel) {
    if (channel <= 0.04045)
        return channel / 12.92;
    else
        return pow((channel + SRGB_ALPHA) / (1.0 + SRGB_ALPHA), 2.4);
}

// Converts a linear rgb color to a srgb color (exact, not approximated)
vec3 rgb_to_srgb(vec3 rgb) {
    return vec3(
        linear_to_srgb(rgb.r),
        linear_to_srgb(rgb.g),
        linear_to_srgb(rgb.b)
    );
}

// Converts a srgb color to a linear rgb color (exact, not approximated)
vec3 srgb_to_rgb(vec3 srgb) {
    return vec3(
        srgb_to_linear(srgb.r),
        srgb_to_linear(srgb.g),
        srgb_to_linear(srgb.b)
    );
}

// Converts a color from linear RGB to XYZ space
vec3 rgb_to_xyz(vec3 rgb) {
    return RGB_2_XYZ * rgb;
}

// Converts a color from XYZ to linear RGB space
vec3 xyz_to_rgb(vec3 xyz) {
    return XYZ_2_RGB * xyz;
}

// Converts a color from XYZ to xyY space (Y is luminosity)
vec3 xyz_to_xyY(vec3 xyz) {
    float Y = xyz.y;
    float x = xyz.x / (xyz.x + xyz.y + xyz.z);
    float y = xyz.y / (xyz.x + xyz.y + xyz.z);
    return vec3(x, y, Y);
}

// Converts a color from xyY space to XYZ space
vec3 xyY_to_xyz(vec3 xyY) {
    float Y = xyY.z;
    float x = Y * xyY.x / xyY.y;
    float z = Y * (1.0 - xyY.x - xyY.y) / xyY.y;
    return vec3(x, Y, z);
}

// Converts a color from linear RGB to xyY space
vec3 rgb_to_xyY(vec3 rgb) {
    vec3 xyz = rgb_to_xyz(rgb);
    return xyz_to_xyY(xyz);
}

// Converts a color from xyY space to linear RGB
vec3 xyY_to_rgb(vec3 xyY) {
    vec3 xyz = xyY_to_xyz(xyY);
    return xyz_to_rgb(xyz);
}

// Converts a value from linear RGB to HCV (Hue, Chroma, Value)
vec3 rgb_to_hcv(vec3 rgb)
{
    // Based on work by Sam Hocevar and Emil Persson
    vec4 P = (rgb.g < rgb.b) ? vec4(rgb.bg, -1.0, 2.0/3.0) : vec4(rgb.gb, 0.0, -1.0/3.0);
    vec4 Q = (rgb.r < P.x) ? vec4(P.xyw, rgb.r) : vec4(rgb.r, P.yzx);
    float C = Q.x - min(Q.w, Q.y);
    float H = abs((Q.w - Q.y) / (6.0 * C + HCV_EPSILON) + Q.z);
    return vec3(H, C, Q.x);
}

// Converts from pure Hue to linear RGB
vec3 hue_to_rgb(float hue)
{
    float R = abs(hue * 6.0 - 3.0) - 1.0;
    float G = 2.0 - abs(hue * 6.0 - 2.0);
    float B = 2.0 - abs(hue * 6.0 - 4.0);
    return saturate(vec3(R,G,B));
}

// Converts from HSV to linear RGB
vec3 hsv_to_rgb(vec3 hsv)
{
    vec3 rgb = hue_to_rgb(hsv.x);
    return ((rgb - 1.0) * hsv.y + 1.0) * hsv.z;
}

// Converts from HSL to linear RGB
vec3 hsl_to_rgb(vec3 hsl)
{
    vec3 rgb = hue_to_rgb(hsl.x);
    float C = (1.0 - abs(2.0 * hsl.z - 1.0)) * hsl.y;
    return (rgb - 0.5) * C + hsl.z;
}

// Converts from HCY to linear RGB
vec3 hcy_to_rgb(vec3 hcy)
{
    const vec3 HCYwts = vec3(0.299, 0.587, 0.114);
    vec3 RGB = hue_to_rgb(hcy.x);
    float Z = dot(RGB, HCYwts);
    if (hcy.z < Z) {
        hcy.y *= hcy.z / Z;
    } else if (Z < 1.0) {
        hcy.y *= (1.0 - hcy.z) / (1.0 - Z);
    }
    return (RGB - Z) * hcy.y + hcy.z;
}


// Converts from linear RGB to HSV
vec3 rgb_to_hsv(vec3 rgb)
{
    vec3 HCV = rgb_to_hcv(rgb);
    float S = HCV.y / (HCV.z + HCV_EPSILON);
    return vec3(HCV.x, S, HCV.z);
}

// Converts from linear rgb to HSL
vec3 rgb_to_hsl(vec3 rgb)
{
    vec3 HCV = rgb_to_hcv(rgb);
    float L = HCV.z - HCV.y * 0.5;
    float S = HCV.y / (1.0 - abs(L * 2.0 - 1.0) + HSL_EPSILON);
    return vec3(HCV.x, S, L);
}

// Converts from rgb to hcy (Hue, Chroma, Luminance)
vec3 rgb_to_hcy(vec3 rgb)
{
    const vec3 HCYwts = vec3(0.299, 0.587, 0.114);
    // Corrected by David Schaeffer
    vec3 HCV = rgb_to_hcv(rgb);
    float Y = dot(rgb, HCYwts);
    float Z = dot(hue_to_rgb(HCV.x), HCYwts);
    if (Y < Z) {
      HCV.y *= Z / (HCY_EPSILON + Y);
    } else {
      HCV.y *= (1.0 - Z) / (HCY_EPSILON + 1.0 - Y);
    }
    return vec3(HCV.x, HCV.y, Y);
}

// RGB to YCbCr, ranges [0, 1]
vec3 rgb_to_ycbcr(vec3 rgb) {
    float y = 0.299 * rgb.r + 0.587 * rgb.g + 0.114 * rgb.b;
    float cb = (rgb.b - y) * 0.565;
    float cr = (rgb.r - y) * 0.713;

    return vec3(y, cb, cr);
}

// YCbCr to RGB
vec3 ycbcr_to_rgb(vec3 yuv) {
    return vec3(
        yuv.x + 1.403 * yuv.z,
        yuv.x - 0.344 * yuv.y - 0.714 * yuv.z,
        yuv.x + 1.770 * yuv.y
    );
}

// Additional conversions converting to rgb first and then to the desired
// color space.

// To srgb
vec3 xyz_to_srgb(vec3 xyz)  { return rgb_to_srgb(xyz_to_rgb(xyz)); }
vec3 xyY_to_srgb(vec3 xyY)  { return rgb_to_srgb(xyY_to_rgb(xyY)); }
vec3 hue_to_srgb(float hue) { return rgb_to_srgb(hue_to_rgb(hue)); }
vec3 hsv_to_srgb(vec3 hsv)  { return rgb_to_srgb(hsv_to_rgb(hsv)); }
vec3 hsl_to_srgb(vec3 hsl)  { return rgb_to_srgb(hsl_to_rgb(hsl)); }
vec3 hcy_to_srgb(vec3 hcy)  { return rgb_to_srgb(hcy_to_rgb(hcy)); }
vec3 ycbcr_to_srgb(vec3 yuv)  { return rgb_to_srgb(ycbcr_to_rgb(yuv)); }

// To xyz
vec3 srgb_to_xyz(vec3 srgb) { return rgb_to_xyz(srgb_to_rgb(srgb)); }
vec3 hue_to_xyz(float hue)  { return rgb_to_xyz(hue_to_rgb(hue)); }
vec3 hsv_to_xyz(vec3 hsv)   { return rgb_to_xyz(hsv_to_rgb(hsv)); }
vec3 hsl_to_xyz(vec3 hsl)   { return rgb_to_xyz(hsl_to_rgb(hsl)); }
vec3 hcy_to_xyz(vec3 hcy)   { return rgb_to_xyz(hcy_to_rgb(hcy)); }
vec3 ycbcr_to_xyz(vec3 yuv)   { return rgb_to_xyz(ycbcr_to_rgb(yuv)); }

// To xyY
vec3 srgb_to_xyY(vec3 srgb) { return rgb_to_xyY(srgb_to_rgb(srgb)); }
vec3 hue_to_xyY(float hue)  { return rgb_to_xyY(hue_to_rgb(hue)); }
vec3 hsv_to_xyY(vec3 hsv)   { return rgb_to_xyY(hsv_to_rgb(hsv)); }
vec3 hsl_to_xyY(vec3 hsl)   { return rgb_to_xyY(hsl_to_rgb(hsl)); }
vec3 hcy_to_xyY(vec3 hcy)   { return rgb_to_xyY(hcy_to_rgb(hcy)); }
vec3 ycbcr_to_xyY(vec3 yuv)   { return rgb_to_xyY(ycbcr_to_rgb(yuv)); }

// To HCV
vec3 srgb_to_hcv(vec3 srgb) { return rgb_to_hcv(srgb_to_rgb(srgb)); }
vec3 xyz_to_hcv(vec3 xyz)   { return rgb_to_hcv(xyz_to_rgb(xyz)); }
vec3 xyY_to_hcv(vec3 xyY)   { return rgb_to_hcv(xyY_to_rgb(xyY)); }
vec3 hue_to_hcv(float hue)  { return rgb_to_hcv(hue_to_rgb(hue)); }
vec3 hsv_to_hcv(vec3 hsv)   { return rgb_to_hcv(hsv_to_rgb(hsv)); }
vec3 hsl_to_hcv(vec3 hsl)   { return rgb_to_hcv(hsl_to_rgb(hsl)); }
vec3 hcy_to_hcv(vec3 hcy)   { return rgb_to_hcv(hcy_to_rgb(hcy)); }
vec3 ycbcr_to_hcv(vec3 yuv)   { return rgb_to_hcy(ycbcr_to_rgb(yuv)); }

// To HSV
vec3 srgb_to_hsv(vec3 srgb) { return rgb_to_hsv(srgb_to_rgb(srgb)); }
vec3 xyz_to_hsv(vec3 xyz)   { return rgb_to_hsv(xyz_to_rgb(xyz)); }
vec3 xyY_to_hsv(vec3 xyY)   { return rgb_to_hsv(xyY_to_rgb(xyY)); }
vec3 hue_to_hsv(float hue)  { return rgb_to_hsv(hue_to_rgb(hue)); }
vec3 hsl_to_hsv(vec3 hsl)   { return rgb_to_hsv(hsl_to_rgb(hsl)); }
vec3 hcy_to_hsv(vec3 hcy)   { return rgb_to_hsv(hcy_to_rgb(hcy)); }
vec3 ycbcr_to_hsv(vec3 yuv)   { return rgb_to_hsv(ycbcr_to_rgb(yuv)); }

// To HSL
vec3 srgb_to_hsl(vec3 srgb) { return rgb_to_hsl(srgb_to_rgb(srgb)); }
vec3 xyz_to_hsl(vec3 xyz)   { return rgb_to_hsl(xyz_to_rgb(xyz)); }
vec3 xyY_to_hsl(vec3 xyY)   { return rgb_to_hsl(xyY_to_rgb(xyY)); }
vec3 hue_to_hsl(float hue)  { return rgb_to_hsl(hue_to_rgb(hue)); }
vec3 hsv_to_hsl(vec3 hsv)   { return rgb_to_hsl(hsv_to_rgb(hsv)); }
vec3 hcy_to_hsl(vec3 hcy)   { return rgb_to_hsl(hcy_to_rgb(hcy)); }
vec3 ycbcr_to_hsl(vec3 yuv)   { return rgb_to_hsl(ycbcr_to_rgb(yuv)); }

// To HCY
vec3 srgb_to_hcy(vec3 srgb) { return rgb_to_hcy(srgb_to_rgb(srgb)); }
vec3 xyz_to_hcy(vec3 xyz)   { return rgb_to_hcy(xyz_to_rgb(xyz)); }
vec3 xyY_to_hcy(vec3 xyY)   { return rgb_to_hcy(xyY_to_rgb(xyY)); }
vec3 hue_to_hcy(float hue)  { return rgb_to_hcy(hue_to_rgb(hue)); }
vec3 hsv_to_hcy(vec3 hsv)   { return rgb_to_hcy(hsv_to_rgb(hsv)); }
vec3 hsl_to_hcy(vec3 hsl)   { return rgb_to_hcy(hsl_to_rgb(hsl)); }
vec3 ycbcr_to_hcy(vec3 yuv)   { return rgb_to_hcy(ycbcr_to_rgb(yuv)); }

// YCbCr
vec3 srgb_to_ycbcr(vec3 srgb) { return rgb_to_ycbcr(srgb_to_rgb(srgb)); }
vec3 xyz_to_ycbcr(vec3 xyz)   { return rgb_to_ycbcr(xyz_to_rgb(xyz)); }
vec3 xyY_to_ycbcr(vec3 xyY)   { return rgb_to_ycbcr(xyY_to_rgb(xyY)); }
vec3 hue_to_ycbcr(float hue)  { return rgb_to_ycbcr(hue_to_rgb(hue)); }
vec3 hsv_to_ycbcr(vec3 hsv)   { return rgb_to_ycbcr(hsv_to_rgb(hsv)); }
vec3 hsl_to_ycbcr(vec3 hsl)   { return rgb_to_ycbcr(hsl_to_rgb(hsl)); }
vec3 hcy_to_ycbcr(vec3 hcy)   { return rgb_to_ycbcr(hcy_to_rgb(hcy)); }

// https://bottosson.github.io/posts/oklab
const mat3 kCONEtoLMS = mat3(                
     0.4121656120,  0.2118591070,  0.0883097947,
     0.5362752080,  0.6807189584,  0.2818474174,
     0.0514575653,  0.1074065790,  0.6302613616);
const mat3 kLMStoCONE = mat3(
     4.0767245293, -1.2681437731, -0.0041119885,
    -3.3072168827,  2.6093323231, -0.7034763098,
     0.2307590544, -0.3411344290,  1.7068625689);
// OKlab
vec3 rgb_to_oklab(vec3 color) { return pow(kCONEtoLMS*color, vec3(1.0/3.0)); }
vec3 oklab_to_rgb(vec3 color) { return kLMStoCONE*(color*color*color); }

float blendAdd(float base, float blend) {
    return min(base+blend,1.0);
}
vec3 blendAdd(vec3 base, vec3 blend) {
    return min(base+blend,vec3(1.0));
}
vec3 blendAdd(vec3 base, vec3 blend, float opacity) {
    return (blendAdd(base, blend) * opacity + base * (1.0 - opacity));
}
vec3 blendAverage(vec3 base, vec3 blend) {
    return (base+blend)/2.0;
}
vec3 blendAverage(vec3 base, vec3 blend, float opacity) {
    return (blendAverage(base, blend) * opacity + base * (1.0 - opacity));
}
float blendColorBurn(float base, float blend) {
    return (blend==0.0)?blend:max((1.0-((1.0-base)/blend)),0.0);
}
vec3 blendColorBurn(vec3 base, vec3 blend) {
    return vec3(blendColorBurn(base.r,blend.r),blendColorBurn(base.g,blend.g),blendColorBurn(base.b,blend.b));
}
vec3 blendColorBurn(vec3 base, vec3 blend, float opacity) {
    return (blendColorBurn(base, blend) * opacity + base * (1.0 - opacity));
}
float blendColorDodge(float base, float blend) {
    return (blend==1.0)?blend:min(base/(1.0-blend),1.0);
}
vec3 blendColorDodge(vec3 base, vec3 blend) {
    return vec3(blendColorDodge(base.r,blend.r),blendColorDodge(base.g,blend.g),blendColorDodge(base.b,blend.b));
}
vec3 blendColorDodge(vec3 base, vec3 blend, float opacity) {
    return (blendColorDodge(base, blend) * opacity + base * (1.0 - opacity));
}
float blendDarken(float base, float blend) {
    return min(blend,base);
}
vec3 blendDarken(vec3 base, vec3 blend) {
    return vec3(blendDarken(base.r,blend.r),blendDarken(base.g,blend.g),blendDarken(base.b,blend.b));
}
vec3 blendDarken(vec3 base, vec3 blend, float opacity) {
    return (blendDarken(base, blend) * opacity + base * (1.0 - opacity));
}
vec3 blendDifference(vec3 base, vec3 blend) {
    return abs(base-blend);
}
vec3 blendDifference(vec3 base, vec3 blend, float opacity) {
    return (blendDifference(base, blend) * opacity + base * (1.0 - opacity));
}
vec3 blendExclusion(vec3 base, vec3 blend) {
    return base+blend-2.0*base*blend;
}
vec3 blendExclusion(vec3 base, vec3 blend, float opacity) {
    return (blendExclusion(base, blend) * opacity + base * (1.0 - opacity));
}
float blendReflect(float base, float blend) {
    return (blend==1.0)?blend:min(base*base/(1.0-blend),1.0);
}
vec3 blendReflect(vec3 base, vec3 blend) {
    return vec3(blendReflect(base.r,blend.r),blendReflect(base.g,blend.g),blendReflect(base.b,blend.b));
}
vec3 blendReflect(vec3 base, vec3 blend, float opacity) {
    return (blendReflect(base, blend) * opacity + base * (1.0 - opacity));
}
vec3 blendGlow(vec3 base, vec3 blend) {
    return blendReflect(blend,base);
}
vec3 blendGlow(vec3 base, vec3 blend, float opacity) {
    return (blendGlow(base, blend) * opacity + base * (1.0 - opacity));
}
float blendOverlay(float base, float blend) {
    return base<0.5?(2.0*base*blend):(1.0-2.0*(1.0-base)*(1.0-blend));
}
vec3 blendOverlay(vec3 base, vec3 blend) {
    return vec3(blendOverlay(base.r,blend.r),blendOverlay(base.g,blend.g),blendOverlay(base.b,blend.b));
}
vec3 blendOverlay(vec3 base, vec3 blend, float opacity) {
    return (blendOverlay(base, blend) * opacity + base * (1.0 - opacity));
}
vec3 blendHardLight(vec3 base, vec3 blend) {
    return blendOverlay(blend,base);
}
vec3 blendHardLight(vec3 base, vec3 blend, float opacity) {
    return (blendHardLight(base, blend) * opacity + base * (1.0 - opacity));
}
float blendVividLight(float base, float blend) {
    return (blend<0.5)?blendColorBurn(base,(2.0*blend)):blendColorDodge(base,(2.0*(blend-0.5)));
}
vec3 blendVividLight(vec3 base, vec3 blend) {
    return vec3(blendVividLight(base.r,blend.r),blendVividLight(base.g,blend.g),blendVividLight(base.b,blend.b));
}
vec3 blendVividLight(vec3 base, vec3 blend, float opacity) {
    return (blendVividLight(base, blend) * opacity + base * (1.0 - opacity));
}
float blendHardMix(float base, float blend) {
    return (blendVividLight(base,blend)<0.5)?0.0:1.0;
}
vec3 blendHardMix(vec3 base, vec3 blend) {
    return vec3(blendHardMix(base.r,blend.r),blendHardMix(base.g,blend.g),blendHardMix(base.b,blend.b));
}
vec3 blendHardMix(vec3 base, vec3 blend, float opacity) {
    return (blendHardMix(base, blend) * opacity + base * (1.0 - opacity));
}
float blendLighten(float base, float blend) {
    return max(blend,base);
}
vec3 blendLighten(vec3 base, vec3 blend) {
    return vec3(blendLighten(base.r,blend.r),blendLighten(base.g,blend.g),blendLighten(base.b,blend.b));
}
vec3 blendLighten(vec3 base, vec3 blend, float opacity) {
    return (blendLighten(base, blend) * opacity + base * (1.0 - opacity));
}
float blendLinearBurn(float base, float blend) {
    // Note : Same implementation as BlendSubtractf
    return max(base+blend-1.0,0.0);
}
vec3 blendLinearBurn(vec3 base, vec3 blend) {
    // Note : Same implementation as BlendSubtract
    return max(base+blend-vec3(1.0),vec3(0.0));
}
vec3 blendLinearBurn(vec3 base, vec3 blend, float opacity) {
    return (blendLinearBurn(base, blend) * opacity + base * (1.0 - opacity));
}
float blendLinearDodge(float base, float blend) {
    // Note : Same implementation as BlendAddf
    return min(base+blend,1.0);
}
vec3 blendLinearDodge(vec3 base, vec3 blend) {
    // Note : Same implementation as BlendAdd
    return min(base+blend,vec3(1.0));
}
vec3 blendLinearDodge(vec3 base, vec3 blend, float opacity) {
    return (blendLinearDodge(base, blend) * opacity + base * (1.0 - opacity));
}
float blendLinearLight(float base, float blend) {
    return blend<0.5?blendLinearBurn(base,(2.0*blend)):blendLinearDodge(base,(2.0*(blend-0.5)));
}
vec3 blendLinearLight(vec3 base, vec3 blend) {
    return vec3(blendLinearLight(base.r,blend.r),blendLinearLight(base.g,blend.g),blendLinearLight(base.b,blend.b));
}
vec3 blendLinearLight(vec3 base, vec3 blend, float opacity) {
    return (blendLinearLight(base, blend) * opacity + base * (1.0 - opacity));
}
vec3 blendMultiply(vec3 base, vec3 blend) {
    return base*blend;
}
vec3 blendMultiply(vec3 base, vec3 blend, float opacity) {
    return (blendMultiply(base, blend) * opacity + base * (1.0 - opacity));
}
vec3 blendNegation(vec3 base, vec3 blend) {
    return vec3(1.0)-abs(vec3(1.0)-base-blend);
}
vec3 blendNegation(vec3 base, vec3 blend, float opacity) {
    return (blendNegation(base, blend) * opacity + base * (1.0 - opacity));
}
vec3 blendNormal(vec3 base, vec3 blend) {
    return blend;
}
vec3 blendNormal(vec3 base, vec3 blend, float opacity) {
    return (blendNormal(base, blend) * opacity + base * (1.0 - opacity));
}
vec3 blendPhoenix(vec3 base, vec3 blend) {
    return min(base,blend)-max(base,blend)+vec3(1.0);
}
vec3 blendPhoenix(vec3 base, vec3 blend, float opacity) {
    return (blendPhoenix(base, blend) * opacity + base * (1.0 - opacity));
}
float blendPinLight(float base, float blend) {
    return (blend<0.5)?blendDarken(base,(2.0*blend)):blendLighten(base,(2.0*(blend-0.5)));
}
vec3 blendPinLight(vec3 base, vec3 blend) {
    return vec3(blendPinLight(base.r,blend.r),blendPinLight(base.g,blend.g),blendPinLight(base.b,blend.b));
}
vec3 blendPinLight(vec3 base, vec3 blend, float opacity) {
    return (blendPinLight(base, blend) * opacity + base * (1.0 - opacity));
}
float blendScreen(float base, float blend) {
    return 1.0-((1.0-base)*(1.0-blend));
}
vec3 blendScreen(vec3 base, vec3 blend) {
    return vec3(blendScreen(base.r,blend.r),blendScreen(base.g,blend.g),blendScreen(base.b,blend.b));
}
vec3 blendScreen(vec3 base, vec3 blend, float opacity) {
    return (blendScreen(base, blend) * opacity + base * (1.0 - opacity));
}
float blendSoftLight(float base, float blend) {
    return (blend<0.5)?(2.0*base*blend+base*base*(1.0-2.0*blend)):(sqrt(base)*(2.0*blend-1.0)+2.0*base*(1.0-blend));
}
vec3 blendSoftLight(vec3 base, vec3 blend) {
    return vec3(blendSoftLight(base.r,blend.r),blendSoftLight(base.g,blend.g),blendSoftLight(base.b,blend.b));
}
vec3 blendSoftLight(vec3 base, vec3 blend, float opacity) {
    return (blendSoftLight(base, blend) * opacity + base * (1.0 - opacity));
}
float blendSubstract(float base, float blend) {
    return max(base+blend-1.0,0.0);
}
vec3 blendSubstract(vec3 base, vec3 blend) {
    return max(base+blend-vec3(1.0),vec3(0.0));
}
vec3 blendSubstract(vec3 base, vec3 blend, float opacity) {
    return (blendSubstract(base, blend) * opacity + blend * (1.0 - opacity));
}
float blendSubtract(float base, float blend) {
    return max(base+blend-1.0,0.0);
}
vec3 blendSubtract(vec3 base, vec3 blend) {
    return max(base+blend-vec3(1.0),vec3(0.0));
}
vec3 blendSubtract(vec3 base, vec3 blend, float opacity) {
    return (blendSubtract(base, blend) * opacity + base * (1.0 - opacity));
}
vec3 blend( vec3 base, vec3 blend ){
    switch (postprocess_uniforms.blend_mode) {
    case 1:
        return blendAdd( base, blend );
    case 2:
        return blendAverage( base, blend );
    case 3:
        return blendColorBurn( base, blend );
    case 4:
        return blendColorDodge( base, blend );
    case 5:
        return blendDarken( base, blend );
    case 6:
        return blendDifference( base, blend );
    case 7:
        return blendExclusion( base, blend );
    case 8:
        return blendGlow( base, blend );
    case 9:
        return blendHardLight( base, blend );
    case 10:
        return blendHardMix( base, blend );
    case 11:
        return blendLighten( base, blend );
    case 12:
        return blendLinearBurn( base, blend );
    case 13:
        return blendLinearDodge( base, blend );
    case 14:
        return blendLinearLight( base, blend );
    case 15:
        return blendMultiply( base, blend );
    case 16:
        return blendNegation( base, blend );
    case 17:
        return blendNormal( base, blend );
    case 18:
        return blendOverlay( base, blend );
    case 19:
        return blendPhoenix( base, blend );
    case 20:
        return blendPinLight( base, blend );
    case 21:
        return blendReflect( base, blend );
    case 22:
        return blendScreen( base, blend );
    case 23:
        return blendSoftLight( base, blend );
    case 24:
        return blendSubtract( base, blend );
    case 25:
        return blendVividLight( base, blend );
    default:
        return base + blend;
    }
}
vec3 blend( vec3 base, vec3 blend, float opacity ){
    switch (postprocess_uniforms.blend_mode) {
    case 1:
        return blendAdd( base, blend, opacity );
    case 2:
        return blendAverage( base, blend, opacity );
    case 3:
        return blendColorBurn( base, blend, opacity );
    case 4:
        return blendColorDodge( base, blend, opacity );
    case 5:
        return blendDarken( base, blend, opacity );
    case 6:
        return blendDifference( base, blend, opacity );
    case 7:
        return blendExclusion( base, blend, opacity );
    case 8:
        return blendGlow( base, blend, opacity );
    case 9:
        return blendHardLight( base, blend, opacity );
    case 10:
        return blendHardMix( base, blend, opacity );
    case 11:
        return blendLighten( base, blend, opacity );
    case 12:
        return blendLinearBurn( base, blend, opacity );
    case 13:
        return blendLinearDodge( base, blend, opacity );
    case 14:
        return blendLinearLight( base, blend, opacity );
    case 15:
        return blendMultiply( base, blend, opacity );
    case 16:
        return blendNegation( base, blend, opacity );
    case 17:
        return blendNormal( base, blend, opacity );
    case 18:
        return blendOverlay( base, blend, opacity );
    case 19:
        return blendPhoenix( base, blend, opacity );
    case 20:
        return blendPinLight( base, blend, opacity );
    case 21:
        return blendReflect( base, blend, opacity );
    case 22:
        return blendScreen( base, blend, opacity );
    case 23:
        return blendSoftLight( base, blend, opacity );
    case 24:
        return blendSubtract( base, blend, opacity );
    case 25:
        return blendVividLight( base, blend, opacity );
    default:
        return base * (1. - opacity) + blend * opacity;
    }
}

vec3 toColorSpace(vec3 val) {
    switch (postprocess_uniforms.color_space) {
    case 1:
        return rgb_to_xyz(val);
    case 2:
        return rgb_to_xyY(val);
    case 3:
        return rgb_to_hsl(val);
    case 4:
        return rgb_to_hsv(val);
    case 5:
        return rgb_to_srgb(val);
    case 6:
        return rgb_to_hcy(val);
    case 7:
        return rgb_to_ycbcr(val);
    case 8:
        return rgb_to_oklab(val);
    default:
        return val;
    }
}

vec3 fromColorSpace(vec3 val) {
    switch (postprocess_uniforms.color_space) {
    case 1:
        return xyz_to_rgb(val);
    case 2:
        return xyY_to_rgb(val);
    case 3:
        return hsl_to_rgb(val);
    case 4:
        return hsv_to_rgb(val);
    case 5:
        return srgb_to_rgb(val);
    case 6:
        return hcy_to_rgb(val);
    case 7:
        return ycbcr_to_rgb(val);
    case 8:
        return oklab_to_rgb(val);
    default:
        return val;
    }
}

void main() {
  vec4 in_color = texture(sampler2D(u_color, u_sampler), a_texcoord);

  vec3 color_shifted = toColorSpace(in_color.rgb);
  for (int i = 0; i < 4; i++) {
      color_shifted = blend(
          color_shifted,
          toColorSpace(postprocess_uniforms.color_shift[i].rgb),
          postprocess_uniforms.color_shift[i].a
      );
  }

  color_attachment = vec4(fromColorSpace(color_shifted), in_color.a);
}
