import Foundation

/// Whether to offer Jefe Pro on the way out of a finished walk.
///
/// A pure decision over explicit inputs, for the same reason `WalkAllowance` is
/// one: the rules here are three small conditions that are easy to get subtly
/// wrong and impossible to notice when they are. The offer fires at most once
/// per install, so a mistake does not announce itself — it just silently never
/// happens, or happens at the one moment guaranteed to annoy.
///
/// ## When it fires
///
/// Isaac, 2026-08-08: *"after they finish their first walk in any way… if the
/// user were to press the back arrow to go back to the home screen they should
/// get the offer."*
///
/// So: every exit from a finished walk — closing the notes, or sending or
/// discarding a document built from them, practice run or real one. Notes
/// appearing is the proof moment, where talking visibly becomes written-up
/// work. Offering before that asks someone to buy a promise; waiting for a
/// built document asks too late and misses everyone who only wanted the notes.
///
/// An earlier version fired only after the optional practice walk. That is an
/// opt-in link *underneath* the primary button in onboarding, so the majority
/// of operators — the ones who tap START MY FIRST WALK — would never have seen
/// it at all.
enum FirstWalkOffer {

    /// `true` when the post-walk Pro offer should be raised.
    ///
    /// - Parameters:
    ///   - isPro: already subscribed — there is nothing to offer.
    ///   - produced: the notes the walk produced, if any.
    ///   - alreadyOffered: whether this install has been shown the offer before.
    static func shouldOffer(
        isPro: Bool,
        produced: NotesModel?,
        alreadyOffered: Bool
    ) -> Bool {
        if isPro { return false }
        if alreadyOffered { return false }
        return producedSomething(produced)
    }

    /// Whether a walk actually captured anything.
    ///
    /// No output, no proof. A walk that captured nothing has just visibly
    /// failed in front of the operator, and asking *that* person for money is
    /// the worst available read of the room — it is also the moment they are
    /// most likely to delete the app.
    ///
    /// Mirrors `NotesView`'s own empty-walk condition (`items.isEmpty &&
    /// notes.isEmpty`) deliberately: if the screen tells them the walk came up
    /// empty, this must agree. Note a walk can carry photos and no words, which
    /// still counts as empty here — the photos are on the walk, but nothing was
    /// *written up*, and the write-up is what the offer is selling.
    static func producedSomething(_ produced: NotesModel?) -> Bool {
        guard let produced else { return false }
        return !(produced.items.isEmpty && produced.notes.isEmpty)
    }
}
