//! Outline list surface adapter.

/// Authorized call-site for `measure_trailer_len` (R13).
#[inline]
pub fn outline_measure_trailer_len(envelope: &crate::list_envelope::ListEnvelope) -> usize {
    crate::list_envelope::measure_trailer_len(envelope)
}
