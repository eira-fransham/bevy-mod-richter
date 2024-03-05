use bevy::render::render_resource::ShaderType;
use bytemuck::{Pod, Zeroable};

// minimum limit is 16384:
// https://www.khronos.org/registry/vulkan/specs/1.2-extensions/html/vkspec.html#limits-maxUniformBufferRange
// but https://vulkan.gpuinfo.org/displaydevicelimit.php?name=maxUniformBufferRange&platform=windows
// indicates that a limit of 65536 or higher is more common
const DYNAMIC_UNIFORM_BUFFER_SIZE: wgpu::BufferAddress = 1 << 19;

// https://www.khronos.org/registry/vulkan/specs/1.2-extensions/html/vkspec.html#limits-minUniformBufferOffsetAlignment
pub const DYNAMIC_UNIFORM_BUFFER_ALIGNMENT: u64 = 256;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Zeroable, Pod, ShaderType)]
pub struct UniformBool {
    value: u32,
}

impl UniformBool {
    pub fn new(value: bool) -> UniformBool {
        UniformBool {
            value: value as u32,
        }
    }
}
