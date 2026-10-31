# MakerFoundry 3D (MF-58): wording for the Terms and the Privacy Policy

**Status:** 2026-10-31. **Merged** into `terms.md` (§§ 4 to 9) and `privacy.md` (§§ 2, 3, 6) with
the owner's choices; kept for the record and for the outside services' lines below, which go in
when each service is switched on. Was: a draft for the owner, not yet in the Terms or the Privacy Policy.
MakerFoundry 3D stays switched off for everyone but admins and founding creators (`MF_3D` unset)
until this wording is approved and merged. Merging makes a new version of each document: people
who agreed to the earlier version keep that record, and new sign-ups agree to the new one
(`mf_agreements`). Legal wording is the owner's (docs/start-here.md); a lawyer's read is advised,
above all for the remix rules.

It describes MakerFoundry 3D as built (MF-35 to MF-57, `apps/makerfoundry/src/model3d/`) and the
owner's decisions of 2026-10-30: licences like Thingiverse's and Printables', sliced G-code dropped
from 3MF files, imports resting on the creator's confirmation and the takedown process, the opt-in
showcase, 100 founding creators with early access. Each `[OWNER: …]` is a choice to make before
merging.

## For the Terms of Service

### In § 4, Publishing designs: "designs" includes 3D models

Add at the start of § 4:

> In these terms, a "design" includes a **3D model**: the files to 3D-print something (STL, 3MF,
> OBJ), its CAD sources (STEP) and notes, with its images.

### A new § 5, after § 4 (§§ 5 to 13 become §§ 6 to 14)

> ## 5. 3D models
>
> - **Licences.** You choose a licence for each model: MakerFoundry's own (Personal use, Small
>   business, Commercial), a Creative Commons licence, CC0, a GNU, BSD, Apache, MIT or CERN
>   Open Hardware licence, or All rights reserved. The open licences are their authors' texts,
>   linked from your model's page; they say what others may do with it, and we don't change them.
> - **Remixes.** If your model is made from someone else's, name it as a parent, and choose a
>   licence its licence allows (for example, a remix of a CC BY-SA model must itself be CC BY-SA;
>   a model whose licence forbids adaptations, or keeps all rights, can't be remixed and shared
>   without its maker's permission). MakerFoundry checks the licences you choose against the
>   rules we publish, to help, but **you** are responsible for following the licences of the
>   models you remix. You may always remix your own models.
> - **Models you've shared elsewhere.** If you import models you've published on another site
>   (MakerWorld, Printables, Thingiverse or anywhere), you confirm you made them or have the right
>   to share them here, and that sharing them here doesn't break that site's terms or any licence
>   you gave there. "Originally published on" is what you tell us; we link to it and don't check
>   it. We may remove models whose ownership is disputed (§ 7, Reporting and removing designs).
> - **Your files are rebuilt.** We read every file you upload to check it, measure it and draw
>   its pictures and 3D view. A 3MF file is saved again with only what's needed to print it: its
>   model, pictures and slicer settings. Sliced G-code and anything else in it are left out,
>   because printers run G-code directly. STEP files and notes are offered as downloads only.
> - **Print settings.** The print profile shown on your model (printer, nozzle, layer height,
>   infill, supports, filaments and the like) is read from your 3MF's settings. It's what your
>   file says, shown to help others; we don't promise it prints well on any printer.
> - **Likes.** Signed-in members can like a published model. The number of likes is public; who
>   liked it isn't shown.
> - **Founding creators** may use MakerFoundry 3D before it opens to everyone. [OWNER: confirm
>   this needs no separate terms.]

### A new § 6, Featuring your project on our social pages (after the new § 5)

> ## 6. Featuring your project on our social pages
>
> When you upload a 3D model, post a make of one, or publish models you've imported, you can tick
> **"Feature my project on MakerFoundry's social pages (Facebook, Instagram, YouTube), credited to
> my maker name."** It's unticked unless you tick it, and you can use MakerFoundry the same either
> way.
>
> If you tick it, you give us a non-exclusive, royalty-free permission to post the project's
> images (cropped or resized to fit), its title and a link to it on MakerFoundry's own pages on
> those sites, with a short caption we write that credits you by your **maker name** (never your
> email or anything else about you). We post by hand and choose what to post; ticking the box
> doesn't promise a post.
>
> You can **withdraw** at any time from Account › Featuring your projects. Withdrawing stops new
> posts at once, and we take down posts already made from it within [OWNER: 7] days. Deleting
> your data withdraws everything. Posts on those sites are also governed by their own terms.

### § 7, Reporting and removing designs: 3D models

Add:

> Reports, copyright notices and counter-notices work the same way for 3D models. While a model
> is taken down, its page, files, pictures and 3D view are hidden, and posts we made of it on our
> social pages are taken down. Takedowns count towards repeat infringement whatever kind of
> design they were.

## For the Privacy Policy

### In § 2, What we collect and why: add

> - **3D models you upload or import**: the files and images, and what reading them finds (size,
>   triangles, the print settings listed in the Terms § 5, which program made a 3MF). From a
>   3MF's settings we keep only those print settings, not the rest of the file's text. Imports
>   keep each file's name and folder until it becomes a draft; files not made into drafts, and
>   imported drafts never published, are deleted after 30 days.
> - **Likes**: which models you've liked, and when. Shown only as a count.
> - **Showcase consents**: what you've said we may feature, when you said so, and when you
>   withdrew, and where we posted it. We keep a withdrawn consent's times so we can take down
>   what was posted. [OWNER: confirm how long to keep a withdrawn consent: proposed, until every
>   post from it is taken down, then a year.]
> - **"Originally published on"**: the site, address and date you give for an imported model.
>   We never fetch that address.

### In § 3, Who we share it with: add

> - **Our social pages**: only for projects you asked us to feature (Terms § 6): their images,
>   title, link and your maker name, posted by us on Facebook, Instagram or YouTube.

### In § 6, Your choices and rights: add

> You can withdraw a showcase consent at any time (Account › Featuring your projects). Your
> likes, imports and showcase consents are in your data download, and deleting your data
> deletes your likes and imports and withdraws every consent.

## Before switching on each outside service (issues #28 to #36)
Each of these is built switched off. When the owner switches one on, the Privacy Policy needs its
line first (a new version of the document). Drafts:

- **Email notices (`MF_RESEND_KEY`), § 2 and § 3:** "If you turn on email notices, we keep which
  comments, makes, remixes and follows to tell you about until your daily email goes (and 30 days
  after), and send it through **Resend**, our email provider, which receives your email address and
  the email's text."
- **Thingiverse import (`MF_THINGIVERSE_CLIENT_ID`), § 2:** "If you import from Thingiverse, you
  sign in there and Thingiverse sends us your Thingiverse user name and your things' files and
  details. We don't keep the access it gives us after the import."
- **Posting to our social pages (`MF_FACEBOOK_PAGE_ID`), § 3:** "Projects you asked us to feature
  are sent to **Meta** (Facebook, Instagram) when we post them" (adds to the showcase line above).
- **Virus scanning (`MF_CLAMAV`), STEP previews (`MF_STEP_CONVERTER`) and customizing
  (`MF_SCAD_SERVICE`), § 3:** "Uploaded files are checked for viruses, and STEP and OpenSCAD files
  are converted, by programs we run on our own hosting (Railway); they aren't shared with anyone
  else." [OWNER: true only if these run as MakerFoundry's own Railway services.]
- **API keys, § 2:** "API keys you make: their name, when made and last used. We keep only a
  fingerprint of each key, never the key."
- **Contests, § 2:** "Your contest entries and votes. Votes are counted, never shown by name."
- **Points (`MF_POINTS`), § 2:** nothing new is collected (points are counted from makes and likes
  already listed).

## After approval

1. Merge the wording into `docs/legal/terms.md` and `docs/legal/privacy.md` (new versions).
2. Run the launch checks (MF-59, issue #27), then set `MF_3D=yes`.
