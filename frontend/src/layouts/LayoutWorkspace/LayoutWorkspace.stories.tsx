import { MemoryRouter } from 'react-router';
import { LayoutWorkspace as Component } from './LayoutWorkspace';

export default {
    title: 'layouts/LayoutWorkspace',
    component: Component,
};

export function LayoutWorkspace() {
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
