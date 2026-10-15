// Use case patterns for the sample library.
// (Deliberately non-canonical source: the formatter golden compares against
// the canonical layout; do not "fix" the spacing here. The import and types
// are real so the batch check compiles this file like any model — shapes that
// need cross-surface artifacts (bare policy actors, `requires`, `realizes`)
// live in tests/conformance/usecases/delivery.uc and the unit tests.)
import "library.mox"

/// Iteration 1: a subject, an actor, and the objective doc — nothing else.
usecase HandOverBook1 {
   subject  lib : Library
   actor member : Writer
}

/// Iteration 2a: named occurrences and a fully anchored message.
usecase HandOverBook2a {
    subject lib : Library {
        occurrence reservationReceived;
    }
    actor member:Writer {
        occurrence  reserveRequested ;
    }
    message  reserveBook of Book from member.reserveRequested to lib.reservationReceived;
    first start then reserveBook.source ;
    first reserveBook.source then reserveBook ;
    first reserveBook  then done ;
}

/// Iteration 2b: message-owned occurrences and an end-less message.
usecase HandOverBook2b {
    subject lib : Library {
        occurrence returnBook.source ;
        occurrence returnBook.target;
    }
    message returnBook of Book ;
}

/// Iteration 3: a performed step anchored to a Library operation, ordered by
/// successions over steps and message ends.
usecase HandOverBook3 {
    subject lib:Library {
        perform lookup : getBook ;
        occurrence shelvingCompleted;
    }
    actor member : Writer {
        occurrence reserveRequested ;
        occurrence bookReceived ;
    }
    message reserveBook of Book from member.reserveRequested to lib.lookup ;
    message handOver of Writer from lib.lookup to member.bookReceived ;
    message reshelve of BookCategory from lib.shelvingCompleted to member.reserveRequested ;
    first start then reserveBook.source ;
    first reserveBook.source then reserveBook ;
    first reserveBook then lookup ;
    first lookup then handOver.source ;
    first handOver.source then handOver ;
    first handOver then lib.shelvingCompleted ;
    first lib.shelvingCompleted then reshelve.source ;
    first reshelve.source then reshelve ;
    first reshelve then done ;
}
