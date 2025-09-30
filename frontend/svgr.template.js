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
