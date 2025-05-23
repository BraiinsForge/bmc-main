import { type LengthRange, getLength } from './generics';

import { LoremIpsum } from 'lorem-ipsum';
import { v4 as UUIDv4 } from 'uuid';

export const lorem = new LoremIpsum({
    words: 'lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua ut enim ad minim veniam quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur excepteur sint occaecat cupidatat non proident sunt in culpa qui officia deserunt mollit anim id est laborum sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium totam rem aperiam eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugit sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt neque porro quisquam est qui dolorem ipsum quia dolor sit amet consectetur adipisci velit sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem ut enim ad minima veniam quis nostrum exercitationem ullam corporis suscipit laboriosam nisi ut aliquid ex ea commodi consequatur quis autem vel eum iure reprehenderit qui in ea voluptate velit esse quam nihil molestiae consequatur vel illum qui dolorem eum fugiat quo voluptas nulla pariatur'
        .split(' ')
        .filter(w => w.length >= 4),
});
export const word = (length: LengthRange): string => {
    const len = getLength(length);
    return (
        new Array(len)
            .fill(null)
            // numbers => letters & omit the "0." preffix
            .map(() => Math.random().toString(36).substring(2))
            .join('')
            // cut to requested length
            .slice(0, len)
    );
};
export const string = word;
export function sentence(wordsCount: LengthRange, wordLength: LengthRange = [4, 10]): string {
    const arr = new Array(getLength(wordsCount)).fill(null).map(() => string(wordLength));
    return arr.join(' ');
}

// URL safe characters: https://stackoverflow.com/a/695469
export const id = (name: string, int: number): string => `${name}~${int}`;
export const uuid = UUIDv4;
