import { MemoryRouter } from 'react-router';
import { LayoutPlain as Component } from './LayoutPlain';

export default {
    title: 'layouts/LayoutPlain',
    component: Component,
};

export function LayoutPlain() {
    return (
        <MemoryRouter>
            <Component>
                Lorem ipsum dolor sit amet, consectetur adipisicing elit. Assumenda atque, consequatur cumque dolores
                dolorum in minima molestiae natus, officiis, omnis pariatur quisquam tempore ullam voluptate voluptatem.
                Aliquid dignissimos eaque eveniet?
            </Component>
        </MemoryRouter>
    );
}
