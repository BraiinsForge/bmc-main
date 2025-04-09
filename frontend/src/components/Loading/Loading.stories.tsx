import { Loading as Component, type LoadingProps as Props } from './Loading';

export default {
    title: 'components/Loading',
    component: Component,
};
export const Loading = (args: Props) => (
    <div className="ui-box tint">
        <Component {...args} />
    </div>
);
Loading.args = {
    size: 250,
    active: true,
} as Props;
