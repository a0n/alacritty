// Copyright 2016 Joe Wilm, The Alacritty Project Contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
#version 330 core
layout (location = 0) in vec2 position;

// Cell properties
layout (location = 1) in vec2 gridCoords;

// glyph properties
layout (location = 2) in vec4 glyph;

// uv mapping
layout (location = 3) in vec4 uv;

// text fg color
layout (location = 4) in vec3 textColor;
// Background color
layout (location = 5) in vec4 backgroundColor;

out vec2 TexCoords;
out vec3 fg;
out vec4 bg;

// Terminal properties
uniform vec2 termDim;
uniform vec2 cellDim;

uniform float visualBell;
uniform int backgroundPass;

// Orthographic projection
uniform mat4 projection;
uniform float time;

flat out float vb;
flat out int background;

vec3 rgb2hsv(vec3 rgb) {
 	float Cmax = max(rgb.r, max(rgb.g, rgb.b));
 	float Cmin = min(rgb.r, min(rgb.g, rgb.b));
 	float delta = Cmax - Cmin;

 	vec3 hsv = vec3(0., 0., Cmax);

 	if (Cmax > Cmin) {
 		hsv.y = delta / Cmax;

 		if (rgb.r == Cmax)
 			hsv.x = (rgb.g - rgb.b) / delta;
 		else {
 			if (rgb.g == Cmax)
 				hsv.x = 2. + (rgb.b - rgb.r) / delta;
 			else
 				hsv.x = 4. + (rgb.r - rgb.g) / delta;
 		}
 		hsv.x = fract(hsv.x / 6.);
 	}
 	return hsv;
 }

vec3 hsv2rgb(vec3 c)
{
    vec4 K = vec4(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
    return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
}

void main()
{
    vec2 glyphOffset = glyph.xy;
    vec2 glyphSize = glyph.zw;
    vec2 uvOffset = uv.xy;
    vec2 uvSize = uv.zw;

    // Position of cell from top-left
    vec2 cellPosition = (cellDim) * gridCoords;

    // Invert Y since framebuffer origin is bottom-left
    cellPosition.y = termDim.y - cellPosition.y - cellDim.y;

    vec2 finalPosition;
    if (backgroundPass != 0) {
        cellPosition.y = cellPosition.y;
        finalPosition = cellDim * position + cellPosition;
        gl_Position = projection * vec4(finalPosition.xy, 0.0, 1.0);
        TexCoords = vec2(0, 0);
    } else {
        // Glyphs are offset within their cell; account for y-flip
        vec2 cellOffset = vec2(glyphOffset.x, glyphOffset.y - glyphSize.y);

        // position coordinates are normalized on [0, 1]
        vec2 finalPosition = glyphSize * position + cellPosition + cellOffset;

        gl_Position = projection * vec4(finalPosition.x, finalPosition.y + sin(sin(finalPosition.y) + finalPosition.x * 0.1) * 16.0 * visualBell, 0.0, 1.0);
        TexCoords = uvOffset + vec2(position.x, 1 - position.y) * uvSize;
    }

    /* plasma */
    float r = sin(time * 0.001);
    float t = r * r * 400.0;
    float x = cellPosition.x;
    float y = cellPosition.y;
    float p0 = sin(x * 0.002 + t);
    float p1 = sin(10*(x*0.0005*sin(t*0.5)+y*0.00005*cos(t*0.3))+t);
    float cx = x*0.0005 - 0.5 + 0.5 * sin(t*2);
    float cy = y*0.0005 - 0.5 + 0.5 * cos(t*3.3333);
    float p2 = sin(sqrt(cx*cx+cy*cy)*3+1);
    float p = (p0 + p1 + p2);

    vb = visualBell * 0.0;
    background = backgroundPass;

    vec3 bgColorHsv = rgb2hsv(backgroundColor.rgb / 255.0);
    if (bgColorHsv.x < 0.01)
      bgColorHsv.y += 0.1;
    bgColorHsv.x += p * 0.5;
    vec3 bgColorRgb = hsv2rgb(bgColorHsv);
    bg = vec4(bgColorRgb.rgb, backgroundColor.a);

    vec3 textColorHsv = rgb2hsv(textColor / vec3(255.0, 255.0, 255.0));
    textColorHsv.y += 0.1;
    textColorHsv.x += p * 0.5;
    vec3 textColorRgb = hsv2rgb(textColorHsv);
    fg = textColorRgb;
}
