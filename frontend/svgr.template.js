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

// svgr.template.js
export default function template({ componentName, imports, jsx, exports }, { tpl }) {
    // language=jsx
    return tpl`
${imports}
import { useEffect, useRef, createElement } from 'react';

function ${componentName}(props) {
    const ref = useRef(null);
  
    useEffect(() => {
        const host = ref.current;
        if (host && !host.shadowRoot) {
            const shadow = ref.current.attachShadow({ mode: 'open' });
            const svgOld = ref.current.firstElementChild;

            const svgNew = svgOld.cloneNode(true);
            svgNew.style.display = '';

            shadow.appendChild(svgNew);
            ref.current.removeChild(svgOld);
        }
    }, []);
  
    return createElement('div', {
        ref: ref,
        style: {
            display: 'inline-block', 
            width: props.width,
            height: props.height ?? props.width,
            color: props.color,
            ...props.style 
        },
        className: props.className 
    }, ${jsx});
}

${exports}
`;
}
