//! Create a date-enriched delivery copy without modifying the original image.
fn main() -> photobridge_core::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err(photobridge_core::Error::Invalid(
            "usage: photo-date SOURCE OUTPUT 'YYYY:MM:DD HH:MM:SS' (UTC)".into(),
        ));
    }
    photobridge_pixel::write_photo_date(args[1].as_ref(), args[2].as_ref(), &args[3], 0)
}
