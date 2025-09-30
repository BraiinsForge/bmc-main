// svgr.template.js
export default function template({ componentName, imports, jsx, exports }, { tpl }) {
    // language=jsx
    return tpl`
${imports}
import { useEffect, useRef, createElement } from 'react';

function ${componentName}(props) {
  const host = useRef(null);
  
  useEffect(() => {
    if (host.current && !host.current.shadowRoot) {
      const shadow = host.current.attachShadow({ mode: 'open' });
      const svg = host.current.firstElementChild;
      const clone = svg.cloneNode(true);
      clone.style.display = '';
      shadow.appendChild(clone);
      svg.style.display = 'none';
    }
  }, []);
  
  return createElement('div', {
      ref: host,
      style: {
          display: 'inline-block', 
          width: props.width,
          height: props.height ?? props.width,
          color: props.color,
          ...props.style 
      },
      className: props.className 
  }, ${jsx});
};

${exports}
  `;
}
