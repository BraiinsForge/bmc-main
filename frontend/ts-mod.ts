// Copyright (C) 2025  Braiins Systems s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

declare module '*.svg' {
    import type { SVGProps, FunctionComponent } from 'react';
    const Component: FunctionComponent<SVGProps<SVGSVGElement>>;
    export default Component;
}

// The asset module declarations below come from @rsbuild/core/types.
// Copyright (c) 2023-present ByteDance, Inc. and its affiliates, licensed under MIT.

// Images
declare module '*.jpg' {
    const src: string;
    export default src;
}
declare module '*.jpeg' {
    const src: string;
    export default src;
}
declare module '*.png' {
    const src: string;
    export default src;
}
declare module '*.ico' {
    const src: string;
    export default src;
}

// Fonts
declare module '*.woff' {
    const src: string;
    export default src;
}
declare module '*.woff2' {
    const src: string;
    export default src;
}
declare module '*.eot' {
    const src: string;
    export default src;
}
declare module '*.ttf' {
    const src: string;
    export default src;
}
declare module '*.otf' {
    const src: string;
    export default src;
}

// CSS Modules
declare module '*.scss' {
    const classes: Readonly<Record<string, string>>;
    export default classes;
}
