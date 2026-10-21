import { PageHead } from '../components/bits';
import { Icon } from '../components/Icon';

/** An unknown `#/...` route: say so instead of silently showing search. */
export function NotFound({ path }: { path: string }) {
  return (
    <>
      <PageHead
        title="Page not found"
        sub={
          <>
            There is no page at <code>{`#/${path}`}</code>. The link may be out of date or mistyped.
          </>
        }
      />
      <a className="btn primary" href="#/discover">
        <Icon name="search" />
        Back to search
      </a>
    </>
  );
}
