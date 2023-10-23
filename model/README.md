Contains business entities, such as events and report models.

# Implementation Note:

This crate is very simple and, usually, creating such a small library wouldn't make sense.

Nonetheless, we are keeping the business entities isolated in their own crate to demonstrate the Multi-Tier architectural pattern used in this project,
where the `model` crate contains data (that are central to the problem being solved) that could be transferred across layers -- such as BLL, DAL and Presentation.