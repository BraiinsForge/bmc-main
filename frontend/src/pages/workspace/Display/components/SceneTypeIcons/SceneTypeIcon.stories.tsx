import { SceneTypeIcons as Component, type SceneTypeIconsProps } from './SceneTypeIcons';
import styled from '@emotion/styled';

export default {
    title: 'Display/Components/SceneTypeIcon',
    component: Component,
};

const cases: SceneTypeIconsProps[] = [
    { cloud: true },
    { local: true },
    { night: true },
    { cloud: true, local: true, night: true },
];

const Wrapper = styled.div`
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 16px;
`;
const Row = styled.div`
    display: inline-flex;
    flex-flow: column;
    padding: 16px;
    gap: 8px;
    background-color: var(--cds-layer-01);
`;
const Code = styled.pre`
    display: inline-block;
`;

export function SceneTypeIcon() {
    return (
        <Wrapper
            children={cases.map((props, i) => {
                return (
                    <Row key={i}>
                        <Code
                            children={Object.entries(props)
                                .filter(([_, v]) => typeof v === 'boolean')
                                .map(([k, _]) => `${k}`)
                                .join(', ')}
                        />
                        <Component {...props} />
                    </Row>
                );
            })}
        />
    );
}
